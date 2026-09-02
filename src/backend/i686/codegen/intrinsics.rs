//! i686 SSE/AES/CRC intrinsic emission and x87 FPU math intrinsics.
//!
//! Handles the `emit_intrinsic` trait method for the i686 backend, covering:
//! - Memory fences (lfence, mfence, sfence, pause)
//! - Non-temporal stores (movnti, movntdq, movntpd)
//! - SSE/SSE2 128-bit packed operations
//! - AES-NI encryption/decryption
//! - CRC32 instructions
//! - Frame/return address intrinsics
//! - x87 FPU math (sqrt, fabs) for F32/F64

use super::emit::I686Codegen;
use crate::backend::state::StackSlot;
use crate::backend::traits::ArchCodegen;
use crate::emit;
use crate::ir::reexports::{IntrinsicOp, IrConst, Operand, Value};

impl I686Codegen {
    pub(super) fn emit_intrinsic_impl(
        &mut self,
        dest: &Option<Value>,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        match op {
            // --- Memory fences (same x86 instructions as x86-64) ---
            IntrinsicOp::Lfence => {
                self.state.emit("    lfence");
            }
            IntrinsicOp::Mfence => {
                self.state.emit("    mfence");
            }
            IntrinsicOp::Sfence => {
                self.state.emit("    sfence");
            }
            IntrinsicOp::Pause => {
                self.state.emit("    pause");
            }
            IntrinsicOp::Clflush => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    clflush (%eax)");
            }

            // --- Non-temporal stores ---
            IntrinsicOp::Movnti
            | IntrinsicOp::Movnti64
            | IntrinsicOp::Movntdq
            | IntrinsicOp::Movntpd => {
                self.emit_nontemporal_store(op, dest_ptr, args);
            }

            // --- SSE 128-bit load/store ---
            IntrinsicOp::Loaddqu => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Storedqu => {
                if let Some(ptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&Operand::Value(*ptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // SSE 128-bit binary operations
            IntrinsicOp::Pcmpeqb128
            | IntrinsicOp::Pcmpeqd128
            | IntrinsicOp::Psubusb128
            | IntrinsicOp::Psubsb128
            | IntrinsicOp::Por128
            | IntrinsicOp::Pand128
            | IntrinsicOp::Pxor128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Pcmpeqb128 => "pcmpeqb",
                        IntrinsicOp::Pcmpeqd128 => "pcmpeqd",
                        IntrinsicOp::Psubusb128 => "psubusb",
                        IntrinsicOp::Psubsb128 => "psubsb",
                        IntrinsicOp::Por128 => "por",
                        IntrinsicOp::Pand128 => "pand",
                        IntrinsicOp::Pxor128 => "pxor",
                        _ => unreachable!("unexpected SSE binary op: {:?}", op),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }
            IntrinsicOp::Pmovmskb128 => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.state.emit("    pmovmskb %xmm0, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::SetEpi8 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    punpcklbw %xmm0, %xmm0");
                    self.state.emit("    punpcklwd %xmm0, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::SetEpi32 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // --- CRC32 ---
            IntrinsicOp::Crc32_8
            | IntrinsicOp::Crc32_16
            | IntrinsicOp::Crc32_32
            | IntrinsicOp::Crc32_64 => {
                self.emit_crc32_intrinsic(op, dest, args);
            }

            // --- Frame and return address ---
            IntrinsicOp::FrameAddress => {
                self.state.emit("    movl %ebp, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::ReturnAddress => {
                // On i686, return address is at 4(%ebp) (32-bit stack frame)
                // With FP omission: param_ref(4) computes the correct ESP-relative offset
                let ra = self.param_ref(4);
                emit!(self.state, "    movl {}, %eax", ra);
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::ThreadPointer => {
                // __builtin_thread_pointer(): read TLS base from %gs:0 on i686
                self.state.emit("    movl %gs:0, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }

            // --- GCC local-frame setjmp/longjmp (mirrors the x86-64 lowering
            // in backend/x86/codegen/intrinsics.rs) ---
            //
            // jmp_buf layout (native words, little-endian):
            //   [0] = %ebp   frame pointer of the setjmp frame
            //   [1] = resume label address (returns 1)
            //   [2] = %esp   stack pointer of the setjmp frame
            //
            // Callers must keep values live across __builtin_setjmp in memory;
            // the pipeline marks every alloca volatile in functions containing
            // the intrinsic and disables register allocation for them, so the
            // frame slots are authoritative after the longjmp lands.
            IntrinsicOp::BuiltinSetjmp => {
                let buffer = args.first().expect("BuiltinSetjmp requires a buffer");
                self.operand_to_eax(buffer);
                // EAX holds the buffer pointer; EDX is our only scratch and is
                // dead here (functions with __builtin_setjmp have no register
                // allocation, so no live value can hide in it).
                self.state.emit("    movl %eax, %edx");
                let resume = self.state.fresh_label("builtin_setjmp_resume");
                let done = self.state.fresh_label("builtin_setjmp_done");
                self.state.emit("    movl %ebp, 0(%edx)");
                emit!(self.state, "    movl ${}, %eax", resume);
                self.state.emit("    movl %eax, 4(%edx)");
                self.state.emit("    movl %esp, 8(%edx)");
                self.state.emit("    xorl %eax, %eax");
                self.state.out.emit_jmp_label(&done);
                self.state.out.emit_named_label(&resume);
                self.state.emit("    movl $1, %eax");
                self.state.out.emit_named_label(&done);
                self.state.reg_cache.invalidate_acc();
                if let Some(dest) = dest {
                    self.store_eax_to(dest);
                }
            }
            IntrinsicOp::BuiltinLongjmp => {
                let buffer = args.first().expect("BuiltinLongjmp requires a buffer");
                self.operand_to_eax(buffer);
                // EAX keeps the buffer pointer across the %esp restore, so all
                // three words are read from a stable base.  This call never
                // returns: %esp/%ebp return to the setjmp frame and control
                // jumps to its resume label.
                self.state.emit("    movl 8(%eax), %edx");
                self.state.emit("    movl 4(%eax), %ecx");
                self.state.emit("    movl %edx, %esp");
                self.state.emit("    movl 0(%eax), %edx");
                self.state.emit("    movl %edx, %ebp");
                self.state.emit("    jmp *%ecx");
                self.state.reg_cache.invalidate_all();
            }

            // --- GCC __builtin_apply family ---
            //
            // Save-area layout (i686):
            //   [0]  incoming %eax   (regparm arg 0 — current value at the
            //                        apply_args call, matching GCC's semantics)
            //   [4]  incoming %edx   (regparm arg 1)
            //   [8]  incoming %ecx   (regparm arg 2)
            //   [16..16+N)           caller's stack argument area (N =
            //                        incoming_stack_arg_bytes)
            // Save-area size: 16 + N (ApplyArgsAreaSize).
            IntrinsicOp::ApplyArgsAreaSize => {
                if let Some(d) = dest {
                    emit!(
                        self.state,
                        "    movl ${}, %eax",
                        16 + self.incoming_stack_arg_bytes
                    );
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::SaveApplyArgs => {
                let area_owned: Operand = dest_ptr
                    .as_ref()
                    .map(|v| Operand::Value(*v))
                    .or_else(|| args.first().cloned())
                    .expect("SaveApplyArgs requires an area pointer");
                let area_op = &area_owned;
                // Callee-saved + arg-register preservation.  A frame pointer is
                // guaranteed: ApplyArgs lowers through DynAlloca, and dynamic
                // allocas force %ebp.
                self.state.emit("    pushl %eax");
                self.esp_adjust += 4;
                self.state.emit("    pushl %esi");
                self.esp_adjust += 4;
                self.state.emit("    pushl %edi");
                self.esp_adjust += 4;
                self.operand_to_eax(area_op);
                self.state.emit("    movl %eax, %edi");
                // Snapshot the (possibly regparm) incoming register args.
                self.state.emit("    movl %edx, 4(%edi)");
                self.state.emit("    movl %ecx, 8(%edi)");
                // Original %eax sits at 8(%esp): three pushes, top = saved %edi.
                self.state.emit("    movl 8(%esp), %eax");
                self.state.emit("    movl %eax, 0(%edi)");
                // Copy the caller's stack argument area.
                self.state.emit("    leal 8(%ebp), %esi");
                self.state.emit("    addl $16, %edi");
                emit!(self.state, "    movl ${}, %ecx", self.incoming_stack_arg_bytes);
                self.state.emit("    cld");
                self.state.emit("    rep movsb");
                self.state.emit("    popl %edi");
                self.esp_adjust -= 4;
                self.state.emit("    popl %esi");
                self.esp_adjust -= 4;
                self.state.emit("    popl %eax");
                self.esp_adjust -= 4;
                self.state.reg_cache.invalidate_acc();
            }
            IntrinsicOp::DoBuiltinApply => {
                // args: [func, save_area, result_area, size]
                debug_assert!(args.len() >= 4, "DoBuiltinApply requires 4 operands");
                let tracked = self.esp_adjust;
                // Callee-saved preservation: %esi (area), %edi (staging dest),
                // %ebx (function pointer).  A frame pointer is guaranteed via
                // the DynAlloca areas the lowering emits.
                self.state.emit("    pushl %esi");
                self.esp_adjust += 4;
                self.state.emit("    pushl %edi");
                self.esp_adjust += 4;
                self.state.emit("    pushl %ebx");
                self.esp_adjust += 4;
                self.emit_load_operand(&args[0]); // func
                self.state.emit("    movl %eax, %ebx");
                self.emit_load_operand(&args[1]); // save_area
                self.state.emit("    movl %eax, %esi");
                self.emit_load_operand(&args[3]); // size
                self.state.emit("    movl %eax, %edx");
                // Stage `size` (16-aligned) bytes of stack arguments from the
                // save area.  This %esp adjustment is deliberately NOT tracked:
                // it is undone by the ebp-relative lea below, and no tracked
                // (esp-relative) slot is touched in between — every operand
                // load here is ebp-relative.
                self.state.emit("    leal 15(%edx), %eax");
                self.state.emit("    andl $-16, %eax");
                self.state.emit("    subl %eax, %esp");
                self.state.emit("    movl %edx, %ecx");
                self.state.emit("    leal 16(%esi), %esi"); // skip reg block
                self.state.emit("    movl %esp, %edi");
                self.state.emit("    cld");
                self.state.emit("    rep movsb");
                // Restore the regparm argument registers from the save block.
                self.state.emit("    subl %edx, %esi"); // back to area base
                self.state.emit("    movl 4(%esi), %edx");
                self.state.emit("    movl 8(%esi), %ecx");
                self.state.emit("    movl 0(%esi), %eax");
                self.state.emit("    call *%ebx");
                // Discard the staging region and return %esp to the tracked
                // position: baseline + entry-esp_adjust + the three pushes.
                emit!(
                    self.state,
                    "    leal -{}(%ebp), %esp",
                    self.esp_baseline_offset + tracked + 12
                );
                // Capture the return value: result[0]=%eax, result[4]=%edx.
                // (x87 returns are not captured; __builtin_return consumers on
                // i686 read the integer half exactly like GCC's ax/dx block.)
                self.state.emit("    pushl %edx");
                self.esp_adjust += 4;
                self.state.emit("    pushl %eax");
                self.esp_adjust += 4;
                self.emit_load_operand(&args[2]); // result_area
                self.state.emit("    movl %eax, %edi");
                self.state.emit("    popl %eax");
                self.esp_adjust -= 4;
                self.state.emit("    movl %eax, 0(%edi)");
                self.state.emit("    popl %eax");
                self.esp_adjust -= 4;
                self.state.emit("    movl %eax, 4(%edi)");
                self.state.emit("    popl %ebx");
                self.esp_adjust -= 4;
                self.state.emit("    popl %edi");
                self.esp_adjust -= 4;
                self.state.emit("    popl %esi");
                self.esp_adjust -= 4;
                self.state.reg_cache.invalidate_all();
            }
            IntrinsicOp::RestoreApplyResult => {
                let block = args.first().expect("RestoreApplyResult requires a block");
                self.operand_to_eax(block);
                self.state.emit("    movl %eax, %esi");
                self.state.emit("    movl 0(%esi), %eax");
                self.state.emit("    movl 4(%esi), %edx");
                self.state.reg_cache.invalidate_all();
            }

            // --- Floating-point intrinsics via x87 FPU ---
            IntrinsicOp::SqrtF64 => {
                self.emit_f64_unary_x87(&args[0], "fsqrt", dest);
            }
            IntrinsicOp::SqrtF32 => {
                self.emit_f32_load_to_x87(&args[0]);
                self.state.emit("    fsqrt");
                self.emit_f32_store_from_x87(dest);
            }
            // S11: directed-rounding scalar intrinsics. simplify.rs
            // rewrites floor/ceil/trunc/rint/nearbyint/roundeven into
            // RoundScalar* intrinsics unconditionally (required for glibc
            // self-hosting on x86-64), but the i686 backend had no arm — the
            // `_ => {}` fallthrough silently dropped the instruction and the
            // result slot was read unwritten (float-floor: floor(d) vanished,
            // (int)garbage != 1023 -> abort at every opt level).
            //
            // x87 lowering: `frndint` rounds per the FPU control word's RC
            // field (bits 10-11). floor/ceil/trunc need a transient RC
            // switch (01 down / 10 up / 11 chop), roundeven forces 00
            // (nearest-even), rint/nearbyint use the ambient mode. The
            // original CW is kept in %ax while the modified word lives in
            // the dynamic 4-byte scratch, so one restore suffices; the
            // scratch window never overlaps a frame-slot reference (arg is
            // loaded before `subl $4`, result stored after `addl $4`), so
            // the transient %esp shift is safe in both frame modes.
            IntrinsicOp::RoundScalarF64(imm) => {
                self.emit_f64_load_to_x87(&args[0]);
                self.emit_x87_frndint_with_mode(*imm);
                if let Some(d) = dest {
                    self.emit_f64_store_from_x87(d);
                } else {
                    self.state.emit("    fstp %st(0)");
                }
            }
            IntrinsicOp::RoundScalarF32(imm) => {
                self.emit_f32_load_to_x87(&args[0]);
                self.emit_x87_frndint_with_mode(*imm);
                self.emit_f32_store_from_x87(dest);
            }
            IntrinsicOp::FabsF64 => {
                self.emit_f64_unary_x87(&args[0], "fabs", dest);
            }
            IntrinsicOp::FabsF32 => {
                self.emit_f32_load_to_x87(&args[0]);
                self.state.emit("    fabs");
                self.emit_f32_store_from_x87(dest);
            }
            IntrinsicOp::CopysignF64 => {
                self.emit_f64_copysign(dest, &args[0], &args[1]);
            }
            IntrinsicOp::CopysignF32 => {
                self.emit_f32_copysign(dest, &args[0], &args[1]);
            }
            // Fused scalar FMA (simplify.rs folds the fma/fmaf libcalls ONLY
            // when the target has FMA3, so reaching this arm implies -mfma on
            // i686). VFMADD231 semantics: dst = src1(vvvv) * src2(r/m) + dst
            // => stage c (acc) into %xmm0, a into %xmm1, b into %xmm2:
            // xmm0 = a * b + c, the C99 fma(a,b,c) with single rounding.
            IntrinsicOp::FmaScalarF32 | IntrinsicOp::FmaScalarF64 => {
                let is_f64 = matches!(op, IntrinsicOp::FmaScalarF64);
                if is_f64 {
                    self.emit_f64_scalar_bits_to_xmm(&args[2], "xmm0");
                    self.emit_f64_scalar_bits_to_xmm(&args[0], "xmm1");
                    self.emit_f64_scalar_bits_to_xmm(&args[1], "xmm2");
                } else {
                    self.emit_f32_scalar_bits_to_xmm(&args[2], "xmm0");
                    self.emit_f32_scalar_bits_to_xmm(&args[0], "xmm1");
                    self.emit_f32_scalar_bits_to_xmm(&args[1], "xmm2");
                }
                if is_f64 {
                    self.state.emit("    vfmadd231sd %xmm2, %xmm1, %xmm0");
                } else {
                    self.state.emit("    vfmadd231ss %xmm2, %xmm1, %xmm0");
                }
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    if let Some(slot) = self.state.get_slot(d.0) {
                        let sr = self.slot_ref(slot);
                        if is_f64 {
                            emit!(self.state, "    movsd %xmm0, {}", sr);
                        } else {
                            emit!(self.state, "    movss %xmm0, {}", sr);
                        }
                    }
                }
            }
            IntrinsicOp::F128Copysign => {
                self.emit_f128_copysign(dest, &args[0], &args[1]);
            }
            IntrinsicOp::F128Neg => {
                self.emit_f128_signop(dest, &args[0], false);
            }
            IntrinsicOp::F128Fabs => {
                self.emit_f128_signop(dest, &args[0], true);
            }
            IntrinsicOp::LDCopysign => {
                self.emit_ld_copysign(dest, &args[0], &args[1]);
            }
            IntrinsicOp::LDFabs => {
                self.emit_ld_fabs(dest, &args[0]);
            }
            // FmaF64x2, FmaF64x4, and reduction intrinsics are x86-64 SSE2/AVX2 intrinsics; not implemented on i686.
            IntrinsicOp::FmaF64x2
            | IntrinsicOp::FmaF64x2Hoisted
            | IntrinsicOp::FmaF64x4
            | IntrinsicOp::FmaF64x4Hoisted
            | IntrinsicOp::BroadcastLoadF64
            | IntrinsicOp::FmaF64x4SIB
            | IntrinsicOp::FmaF64x4HoistedSIB
            | IntrinsicOp::LoadF64x4
            | IntrinsicOp::LoadF64x2
            | IntrinsicOp::LoadI32x8
            | IntrinsicOp::LoadI32x4
            | IntrinsicOp::AddF64x4
            | IntrinsicOp::AddF64x2
            | IntrinsicOp::MulF64x4
            | IntrinsicOp::MulF64x2
            | IntrinsicOp::AddI32x8
            | IntrinsicOp::AddI32x4
            | IntrinsicOp::HorizontalAddF64x4
            | IntrinsicOp::HorizontalAddF64x2
            | IntrinsicOp::HorizontalAddI32x8
            | IntrinsicOp::HorizontalAddI32x4 => {
                // Not reachable on i686 - emit a no-op placeholder.
            }

            // --- AES-NI ---
            IntrinsicOp::Aesenc128
            | IntrinsicOp::Aesenclast128
            | IntrinsicOp::Aesdec128
            | IntrinsicOp::Aesdeclast128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Aesenc128 => "aesenc",
                        IntrinsicOp::Aesenclast128 => "aesenclast",
                        IntrinsicOp::Aesdec128 => "aesdec",
                        IntrinsicOp::Aesdeclast128 => "aesdeclast",
                        _ => unreachable!("AES-NI dispatch matched non-AES op: {:?}", op),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }
            IntrinsicOp::Aesimc128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.state.emit("    aesimc %xmm0, %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Aeskeygenassist128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    let imm = Self::operand_to_imm_i64(&args[1]);
                    self.state
                        .emit_fmt(format_args!("    aeskeygenassist ${}, %xmm0, %xmm0", imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Pclmulqdq128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&args[1]);
                    self.state.emit("    movdqu (%eax), %xmm1");
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pclmulqdq ${}, %xmm1, %xmm0", imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // SSE2 shift-by-immediate operations
            IntrinsicOp::Pslldqi128
            | IntrinsicOp::Psrldqi128
            | IntrinsicOp::Psllqi128
            | IntrinsicOp::Psrlqi128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Pslldqi128 => "pslldq",
                        IntrinsicOp::Psrldqi128 => "psrldq",
                        IntrinsicOp::Psllqi128 => "psllq",
                        IntrinsicOp::Psrlqi128 => "psrlq",
                        _ => unreachable!("unexpected SSE shift-by-immediate op: {:?}", op),
                    };
                    self.emit_sse_unary_imm_128(dptr, args, inst);
                }
            }
            // SSE2 shuffle with immediate
            IntrinsicOp::Pshufd128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_sse_shuffle_imm_128(dptr, args, "pshufd");
                }
            }
            IntrinsicOp::Loadldi128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movq (%eax), %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // SSE2 binary 128-bit operations
            IntrinsicOp::Paddw128
            | IntrinsicOp::Psubw128
            | IntrinsicOp::Pmulhw128
            | IntrinsicOp::Pmaddwd128
            | IntrinsicOp::Pcmpgtw128
            | IntrinsicOp::Pcmpgtb128
            | IntrinsicOp::Paddd128
            | IntrinsicOp::Psubd128
            | IntrinsicOp::Packssdw128
            | IntrinsicOp::Packsswb128
            | IntrinsicOp::Packuswb128
            | IntrinsicOp::Punpcklbw128
            | IntrinsicOp::Punpckhbw128
            | IntrinsicOp::Punpcklwd128
            | IntrinsicOp::Punpckhwd128
            | IntrinsicOp::Paddb128
            | IntrinsicOp::Paddq128
            | IntrinsicOp::Paddsb128
            | IntrinsicOp::Paddsw128
            | IntrinsicOp::Paddusb128
            | IntrinsicOp::Paddusw128
            | IntrinsicOp::Psubb128
            | IntrinsicOp::Psubq128
            | IntrinsicOp::Psubsw128
            | IntrinsicOp::Psubusw128
            | IntrinsicOp::Pandn128
            | IntrinsicOp::Pmullw128
            | IntrinsicOp::Pmulhuw128
            | IntrinsicOp::Pmuludq128
            | IntrinsicOp::Pmuldq128
            | IntrinsicOp::Pmulld128
            | IntrinsicOp::Pmaddubsw128
            | IntrinsicOp::Pcmpeqw128
            | IntrinsicOp::Pcmpgtd128
            | IntrinsicOp::Pavgb128
            | IntrinsicOp::Pavgw128
            | IntrinsicOp::Pmaxub128
            | IntrinsicOp::Pmaxsw128
            | IntrinsicOp::Pminub128
            | IntrinsicOp::Pminsw128
            | IntrinsicOp::Psadbw128
            | IntrinsicOp::Pshufb128
            | IntrinsicOp::Punpckldq128
            | IntrinsicOp::Punpckhdq128
            | IntrinsicOp::Punpcklqdq128
            | IntrinsicOp::Punpckhqdq128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Paddw128 => "paddw",
                        IntrinsicOp::Psubw128 => "psubw",
                        IntrinsicOp::Pmulhw128 => "pmulhw",
                        IntrinsicOp::Pmaddwd128 => "pmaddwd",
                        IntrinsicOp::Pcmpgtw128 => "pcmpgtw",
                        IntrinsicOp::Pcmpgtb128 => "pcmpgtb",
                        IntrinsicOp::Paddd128 => "paddd",
                        IntrinsicOp::Psubd128 => "psubd",
                        IntrinsicOp::Packssdw128 => "packssdw",
                        IntrinsicOp::Packsswb128 => "packsswb",
                        IntrinsicOp::Packuswb128 => "packuswb",
                        IntrinsicOp::Punpcklbw128 => "punpcklbw",
                        IntrinsicOp::Punpckhbw128 => "punpckhbw",
                        IntrinsicOp::Punpcklwd128 => "punpcklwd",
                        IntrinsicOp::Punpckhwd128 => "punpckhwd",
                        IntrinsicOp::Paddb128 => "paddb",
                        IntrinsicOp::Paddq128 => "paddq",
                        IntrinsicOp::Paddsb128 => "paddsb",
                        IntrinsicOp::Paddsw128 => "paddsw",
                        IntrinsicOp::Paddusb128 => "paddusb",
                        IntrinsicOp::Paddusw128 => "paddusw",
                        IntrinsicOp::Psubb128 => "psubb",
                        IntrinsicOp::Psubq128 => "psubq",
                        IntrinsicOp::Psubsw128 => "psubsw",
                        IntrinsicOp::Psubusw128 => "psubusw",
                        IntrinsicOp::Pandn128 => "pandn",
                        IntrinsicOp::Pmullw128 => "pmullw",
                        IntrinsicOp::Pmulhuw128 => "pmulhuw",
                        IntrinsicOp::Pmuludq128 => "pmuludq",
                        IntrinsicOp::Pmuldq128 => "pmuldq",
                        IntrinsicOp::Pmulld128 => "pmulld",
                        IntrinsicOp::Pmaddubsw128 => "pmaddubsw",
                        IntrinsicOp::Pcmpeqw128 => "pcmpeqw",
                        IntrinsicOp::Pcmpgtd128 => "pcmpgtd",
                        IntrinsicOp::Pavgb128 => "pavgb",
                        IntrinsicOp::Pavgw128 => "pavgw",
                        IntrinsicOp::Pmaxub128 => "pmaxub",
                        IntrinsicOp::Pmaxsw128 => "pmaxsw",
                        IntrinsicOp::Pminub128 => "pminub",
                        IntrinsicOp::Pminsw128 => "pminsw",
                        IntrinsicOp::Psadbw128 => "psadbw",
                        IntrinsicOp::Pshufb128 => "pshufb",
                        IntrinsicOp::Punpckldq128 => "punpckldq",
                        IntrinsicOp::Punpckhdq128 => "punpckhdq",
                        IntrinsicOp::Punpcklqdq128 => "punpcklqdq",
                        IntrinsicOp::Punpckhqdq128 => "punpckhqdq",
                        _ => unreachable!("unexpected SSE binary op: {:?}", op),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }

            // SSE2 element shift-by-immediate operations
            IntrinsicOp::Psllwi128
            | IntrinsicOp::Psrlwi128
            | IntrinsicOp::Psrawi128
            | IntrinsicOp::Psradi128
            | IntrinsicOp::Pslldi128
            | IntrinsicOp::Psrldi128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Psllwi128 => "psllw",
                        IntrinsicOp::Psrlwi128 => "psrlw",
                        IntrinsicOp::Psrawi128 => "psraw",
                        IntrinsicOp::Psradi128 => "psrad",
                        IntrinsicOp::Pslldi128 => "pslld",
                        IntrinsicOp::Psrldi128 => "psrld",
                        _ => unreachable!("unexpected SSE element shift op: {:?}", op),
                    };
                    self.emit_sse_unary_imm_128(dptr, args, inst);
                }
            }

            // --- SSE/SSE2 packed float binary operations (128-bit) ---
            // Same load-op-store shape as the integer arm: args are alloca
            // pointers, slots are only 8-aligned so every access uses
            // movdqu/movups (never a folded memory operand — legacy SSE
            // memory forms require 16-byte alignment).
            IntrinsicOp::AddPs128
            | IntrinsicOp::SubPs128
            | IntrinsicOp::MulPs128
            | IntrinsicOp::AddPd128
            | IntrinsicOp::SubPd128
            | IntrinsicOp::MulPd128
            | IntrinsicOp::DivPs128
            | IntrinsicOp::DivPd128
            | IntrinsicOp::MinPs128
            | IntrinsicOp::MaxPs128
            | IntrinsicOp::MinPd128
            | IntrinsicOp::MaxPd128
            | IntrinsicOp::HaddPs128
            | IntrinsicOp::HsubPs128
            | IntrinsicOp::HaddPd128
            | IntrinsicOp::HsubPd128
            | IntrinsicOp::AddsubPs128
            | IntrinsicOp::AddsubPd128
            | IntrinsicOp::UnpcklPs128
            | IntrinsicOp::UnpckhPs128
            | IntrinsicOp::UnpcklPd128
            | IntrinsicOp::UnpckhPd128
            | IntrinsicOp::Movss128
            | IntrinsicOp::Movsd128
            | IntrinsicOp::CvtSs2Sd128
            | IntrinsicOp::CvtSd2Ss128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::AddPs128 => "addps",
                        IntrinsicOp::SubPs128 => "subps",
                        IntrinsicOp::MulPs128 => "mulps",
                        IntrinsicOp::AddPd128 => "addpd",
                        IntrinsicOp::SubPd128 => "subpd",
                        IntrinsicOp::MulPd128 => "mulpd",
                        IntrinsicOp::DivPs128 => "divps",
                        IntrinsicOp::DivPd128 => "divpd",
                        IntrinsicOp::MinPs128 => "minps",
                        IntrinsicOp::MaxPs128 => "maxps",
                        IntrinsicOp::MinPd128 => "minpd",
                        IntrinsicOp::MaxPd128 => "maxpd",
                        IntrinsicOp::HaddPs128 => "haddps",
                        IntrinsicOp::HsubPs128 => "hsubps",
                        IntrinsicOp::HaddPd128 => "haddpd",
                        IntrinsicOp::HsubPd128 => "hsubpd",
                        IntrinsicOp::AddsubPs128 => "addsubps",
                        IntrinsicOp::AddsubPd128 => "addsubpd",
                        IntrinsicOp::UnpcklPs128 => "unpcklps",
                        IntrinsicOp::UnpckhPs128 => "unpckhps",
                        IntrinsicOp::UnpcklPd128 => "unpcklpd",
                        IntrinsicOp::UnpckhPd128 => "unpckhpd",
                        IntrinsicOp::Movss128 => "movss",
                        IntrinsicOp::Movsd128 => "movsd",
                        IntrinsicOp::CvtSs2Sd128 => "cvtss2sd",
                        IntrinsicOp::CvtSd2Ss128 => "cvtsd2ss",
                        _ => unreachable!("unexpected SSE float binary op: {:?}", op),
                    };
                    self.emit_sse_binary_128(dptr, args, inst);
                }
            }

            // 2-op with immediate: cmpps/cmppd/shufps/shufpd take (a, b, imm).
            IntrinsicOp::CmpPs128
            | IntrinsicOp::CmpPd128
            | IntrinsicOp::ShufPs128
            | IntrinsicOp::ShufPd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::CmpPs128 => "cmpps",
                        IntrinsicOp::CmpPd128 => "cmppd",
                        IntrinsicOp::ShufPs128 => "shufps",
                        IntrinsicOp::ShufPd128 => "shufpd",
                        _ => unreachable!("unexpected SSE fp imm op: {:?}", op),
                    };
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&args[1]);
                    self.state.emit("    movdqu (%eax), %xmm1");
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %xmm1, %xmm0", inst, imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // Blend with immediate (SSE4.1): blendps/blendpd/dpps/dppd (a, b, imm).
            IntrinsicOp::BlendPs128
            | IntrinsicOp::BlendPd128
            | IntrinsicOp::DpPs128
            | IntrinsicOp::DpPd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::BlendPs128 => "blendps",
                        IntrinsicOp::BlendPd128 => "blendpd",
                        IntrinsicOp::DpPs128 => "dpps",
                        IntrinsicOp::DpPd128 => "dppd",
                        _ => unreachable!("unexpected SSE blend op: {:?}", op),
                    };
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&args[1]);
                    self.state.emit("    movdqu (%eax), %xmm1");
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %xmm1, %xmm0", inst, imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // blendv (SSE4.1): operands (mask, a, b); the mask is implicit in
            // %xmm0 for the legacy encoding — load it FIRST, then a (dst),
            // then b (src): result = mask ? b : a per lane.
            IntrinsicOp::BlendvPs128 | IntrinsicOp::BlendvPd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::BlendvPs128) {
                        "blendvps"
                    } else {
                        "blendvpd"
                    };
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0"); // mask
                    self.operand_to_eax(&args[1]);
                    self.state.emit("    movdqu (%eax), %xmm1"); // a (dst)
                    self.operand_to_eax(&args[2]);
                    self.state.emit("    movdqu (%eax), %xmm2"); // b (src)
                    self.state
                        .emit_fmt(format_args!("    {} %xmm2, %xmm1", inst));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm1, (%eax)");
                }
            }

            // Pblendvb (SSE4.1): args = (a, b, mask); mask implicit in %xmm0.
            IntrinsicOp::Pblendvb128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[2]);
                    self.state.emit("    movdqu (%eax), %xmm0"); // mask
                    self.operand_to_eax(&args[1]);
                    self.state.emit("    movdqu (%eax), %xmm1"); // b (src)
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm2"); // a (dst)
                    self.state.emit("    pblendvb %xmm1, %xmm2");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm2, (%eax)");
                }
            }

            // Pblendw (SSE4.1): _mm_blend_epi16(a, b, imm8) → pblendw $imm8, %xmm1, %xmm0.
            IntrinsicOp::Pblendw128 => {
                if let Some(dptr) = dest_ptr {
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&args[1]);
                    self.state.emit("    movdqu (%eax), %xmm1");
                    self.state
                        .emit_fmt(format_args!("    pblendw ${}, %xmm1, %xmm0", imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // Palignr (SSSE3): (a, b, imm) → palignr $imm, %xmm1, %xmm0.
            IntrinsicOp::Palignr128 => {
                if let Some(dptr) = dest_ptr {
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&args[1]);
                    self.state.emit("    movdqu (%eax), %xmm1");
                    self.state
                        .emit_fmt(format_args!("    palignr ${}, %xmm1, %xmm0", imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // --- SSE packed float unary operations ---
            IntrinsicOp::SqrtPs128
            | IntrinsicOp::SqrtPd128
            | IntrinsicOp::RcpPs128
            | IntrinsicOp::RsqrtPs128
            | IntrinsicOp::Movddup128
            | IntrinsicOp::Movsldup128
            | IntrinsicOp::Movshdup128
            | IntrinsicOp::CvtPs2Pd128
            | IntrinsicOp::CvtPd2Ps128
            | IntrinsicOp::CvtPs2Ep32_128
            | IntrinsicOp::CvttPs2Ep32_128
            | IntrinsicOp::CvtEp32ToPs128
            | IntrinsicOp::CvtPd2Ep32_128
            | IntrinsicOp::CvttPd2Ep32_128
            | IntrinsicOp::CvtEp32ToPd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::SqrtPs128 => "sqrtps",
                        IntrinsicOp::SqrtPd128 => "sqrtpd",
                        IntrinsicOp::RcpPs128 => "rcpps",
                        IntrinsicOp::RsqrtPs128 => "rsqrtps",
                        IntrinsicOp::Movddup128 => "movddup",
                        IntrinsicOp::Movsldup128 => "movsldup",
                        IntrinsicOp::Movshdup128 => "movshdup",
                        IntrinsicOp::CvtPs2Pd128 => "cvtps2pd",
                        IntrinsicOp::CvtPd2Ps128 => "cvtpd2ps",
                        IntrinsicOp::CvtPs2Ep32_128 => "cvtps2dq",
                        IntrinsicOp::CvttPs2Ep32_128 => "cvttps2dq",
                        IntrinsicOp::CvtEp32ToPs128 => "cvtdq2ps",
                        IntrinsicOp::CvtPd2Ep32_128 => "cvtpd2dq",
                        IntrinsicOp::CvttPd2Ep32_128 => "cvttpd2dq",
                        IntrinsicOp::CvtEp32ToPd128 => "cvtdq2pd",
                        _ => unreachable!("unexpected SSE float unary op: {:?}", op),
                    };
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm0, %xmm0", inst));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // Round with immediate (SSE4.1): (a, imm).
            IntrinsicOp::RoundPs128 | IntrinsicOp::RoundPd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::RoundPs128) {
                        "roundps"
                    } else {
                        "roundpd"
                    };
                    let imm = Self::operand_to_imm_i64(&args[1]);
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %xmm0, %xmm0", inst, imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // Variable-count word shifts (count in the second xmm): (v, count).
            IntrinsicOp::Psllw128 | IntrinsicOp::Psrlw128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Psllw128) {
                        "psllw"
                    } else {
                        "psrlw"
                    };
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&args[1]);
                    self.state.emit("    movdqu (%eax), %xmm1");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm1, %xmm0", inst));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // SSSE3 horizontal adds: (a, b) → phaddw/phaddd %xmm1, %xmm0.
            IntrinsicOp::Phaddw128 | IntrinsicOp::Phaddd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Phaddw128) {
                        "phaddw"
                    } else {
                        "phaddd"
                    };
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&args[1]);
                    self.state.emit("    movdqu (%eax), %xmm1");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm1, %xmm0", inst));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // SSE4.1 widening conversions are unary (pmovzxbw/pmovzxwd).
            IntrinsicOp::Pmovzxbw128 | IntrinsicOp::Pmovzxwd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Pmovzxbw128) {
                        "pmovzxbw"
                    } else {
                        "pmovzxwd"
                    };
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm0, %xmm0", inst));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // SSSE3 absolute value: pabsb/pabsw/pabsd are UNARY in AT&T form.
            IntrinsicOp::Pabsb128 | IntrinsicOp::Pabsw128 | IntrinsicOp::Pabsd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Pabsb128 => "pabsb",
                        IntrinsicOp::Pabsw128 => "pabsw",
                        IntrinsicOp::Pabsd128 => "pabsd",
                        _ => unreachable!("unexpected SSE abs op: {:?}", op),
                    };
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm0, %xmm0", inst));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // Scalar GPR conversions: cvtsi2ss/cvtsi2sd (a, i) — i is a plain
            // 32-bit integer value on i686, staged through %eax.
            IntrinsicOp::CvtSi2Ss128 | IntrinsicOp::CvtSi2Sd128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::CvtSi2Ss128) {
                        "cvtsi2ss"
                    } else {
                        "cvtsi2sd"
                    };
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&args[1]);
                    self.state
                        .emit_fmt(format_args!("    {} %eax, %xmm0", inst));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // Scalar GPR results: movemask, extractps, cvtss2si/cvtsd2si.
            IntrinsicOp::MovemaskPs128 | IntrinsicOp::MovemaskPd128 => {
                let inst = if matches!(op, IntrinsicOp::MovemaskPs128) {
                    "movmskps"
                } else {
                    "movmskpd"
                };
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.state
                    .emit_fmt(format_args!("    {} %xmm0, %eax", inst));
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::ExtractPs128 => {
                // args: (a, imm) — extractps $imm, %xmm0, %eax
                let imm = Self::operand_to_imm_i64(&args[1]);
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.state
                    .emit_fmt(format_args!("    extractps ${}, %xmm0, %eax", imm));
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::CvtSs2Si128 | IntrinsicOp::CvtSd2Si128 => {
                let inst = if matches!(op, IntrinsicOp::CvtSs2Si128) {
                    "cvtss2si"
                } else {
                    "cvtsd2si"
                };
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.state
                    .emit_fmt(format_args!("    {} %xmm0, %eax", inst));
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }

            // 128-bit zero: pxor is the canonical zeroing idiom.
            IntrinsicOp::Setzero128 => {
                if let Some(dptr) = dest_ptr {
                    self.state.emit("    pxor %xmm0, %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }

            // PTEST: ZF=1 when AND(a,b)==0. _mm_testz_si128 returns that ZF.
            IntrinsicOp::Testz128 => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.operand_to_eax(&args[1]);
                self.state.emit("    movdqu (%eax), %xmm1");
                self.state.emit("    ptest %xmm1, %xmm0");
                self.state.emit("    sete %al");
                self.state.emit("    movzbl %al, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }

            // Reinterpret cast: the operand pointer IS the result — nothing
            // to emit (the lowering hands the pointer through).
            IntrinsicOp::CastReinterpret128 => {}

            // rdtsc/rdtscp: EDX:EAX -> I64 dest slot pair (GCC i386 returns
            // unsigned long long in %edx:%eax; the slot stores low=EAX,
            // high=EDX). rdtscp additionally clobbers %ecx (IA32_TSC_AUX).
            IntrinsicOp::Rdtsc | IntrinsicOp::Rdtscp => {
                if matches!(op, IntrinsicOp::Rdtscp) {
                    self.state.emit("    rdtscp");
                } else {
                    self.state.emit("    rdtsc");
                }
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    if let Some(slot) = self.state.get_slot(d.0) {
                        let sr0 = self.slot_ref(slot);
                        let sr4 = self.slot_ref_offset(slot, 4);
                        emit!(self.state, "    movl %eax, {}", sr0);
                        emit!(self.state, "    movl %edx, {}", sr4);
                    }
                }
            }

            // --- SSE2 set/insert/extract/convert ---
            IntrinsicOp::SetEpi16 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movd %eax, %xmm0");
                    self.state.emit("    punpcklwd %xmm0, %xmm0");
                    self.state.emit("    pshufd $0, %xmm0, %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Pinsrw128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_ecx(&args[1]);
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pinsrw ${}, %ecx, %xmm0", imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Pextrw128 => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                let imm = Self::operand_to_imm_i64(&args[1]);
                self.state
                    .emit_fmt(format_args!("    pextrw ${}, %xmm0, %eax", imm));
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::Pinsrd128 => {
                // Insert 32-bit value at lane: pinsrd $imm, %eax, %xmm0 (SSE4.1)
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_ecx(&args[1]);
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pinsrd ${}, %ecx, %xmm0", imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Pextrd128 => {
                // Extract 32-bit value at lane: pextrd $imm, %xmm0, %eax (SSE4.1)
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                let imm = Self::operand_to_imm_i64(&args[1]);
                self.state
                    .emit_fmt(format_args!("    pextrd ${}, %xmm0, %eax", imm));
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::Pinsrb128 => {
                // Insert 8-bit value at lane: pinsrb $imm, %eax, %xmm0 (SSE4.1)
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_ecx(&args[1]);
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.state
                        .emit_fmt(format_args!("    pinsrb ${}, %ecx, %xmm0", imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Pextrb128 => {
                // Extract 8-bit value at lane: pextrb $imm, %xmm0, %eax (SSE4.1)
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                let imm = Self::operand_to_imm_i64(&args[1]);
                self.state
                    .emit_fmt(format_args!("    pextrb ${}, %xmm0, %eax", imm));
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::Pinsrq128 => {
                // i686 (no 64-bit GPRs): PINSRQ's r64 form is unavailable, but
                // the lane is exactly two dwords — replace it with two PINSRD
                // (SSE4.1, 32-bit register form). Value arrives as the
                // eax(lo):edx(hi) pair; the vector is staged whole in %xmm0 so
                // in-place (dest == src) inserts stay consistent.
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.emit_load_acc_pair(&args[1]);
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.state.emit("    movl %eax, %ecx");
                    emit!(self.state, "    pinsrd ${}, %ecx, %xmm0", 2 * imm);
                    self.state.emit("    movl %edx, %ecx");
                    emit!(self.state, "    pinsrd ${}, %ecx, %xmm0", 2 * imm + 1);
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Pextrq128 => {
                // i686: PEXTRQ's r64 result form is unavailable — extract the
                // lane's two dwords with PEXTRD into the eax(lo):edx(hi) pair.
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                let imm = Self::operand_to_imm_i64(&args[1]);
                emit!(self.state, "    pextrd ${}, %xmm0, %eax", 2 * imm);
                emit!(self.state, "    pextrd ${}, %xmm0, %edx", 2 * imm + 1);
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.emit_store_acc_pair(d);
                }
            }
            IntrinsicOp::Storeldi128 => {
                if let Some(ptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&Operand::Value(*ptr));
                    self.state.emit("    movq %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Cvtsi128Si32 => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }
            IntrinsicOp::Cvtsi32Si128 => {
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movd %eax, %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Cvtsi128Si64 => {
                // i686: full 64-bit extract = the lane-0 Pextrq pair. The
                // historic arm moved only the low dword and dropped the high
                // one (cvtsi64 FAIL: the u64 result lost its high half).
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                self.state.emit("    pextrd $1, %xmm0, %edx");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.emit_store_acc_pair(d);
                }
            }
            IntrinsicOp::Pshuflw128 | IntrinsicOp::Pshufhw128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Pshuflw128 => "pshuflw",
                        IntrinsicOp::Pshufhw128 => "pshufhw",
                        _ => unreachable!("unexpected SSE shuffle op: {:?}", op),
                    };
                    self.emit_sse_shuffle_imm_128(dptr, args, inst);
                }
            }

            // Register-based vector intrinsics (x86-64-specific, not implemented for i686)
            IntrinsicOp::VecLoadF64x4
            | IntrinsicOp::VecLoadF64x2
            | IntrinsicOp::VecLoadI32x8
            | IntrinsicOp::VecLoadI32x4
            | IntrinsicOp::VecAddF64x4
            | IntrinsicOp::VecAddF64x2
            | IntrinsicOp::VecMulF64x4
            | IntrinsicOp::VecMulF64x2
            | IntrinsicOp::VecAddI32x8
            | IntrinsicOp::VecAddI32x4
            | IntrinsicOp::VecHorizontalAddF64x4
            | IntrinsicOp::VecHorizontalAddF64x2
            | IntrinsicOp::VecHorizontalAddI32x8
            | IntrinsicOp::VecHorizontalAddI32x4
            | IntrinsicOp::VecZeroF64x4
            | IntrinsicOp::VecZeroF64x2
            | IntrinsicOp::VecZeroI32x8
            | IntrinsicOp::VecZeroI32x4
            | IntrinsicOp::VecLoadWidenI32ToI64x2
            | IntrinsicOp::VecAddI64x2
            | IntrinsicOp::VecMulI64x2
            | IntrinsicOp::VecHorizontalAddI64x2
            | IntrinsicOp::VecZeroI64x2
            | IntrinsicOp::VecMulI32x4
            | IntrinsicOp::VecBroadcastI32x4
            | IntrinsicOp::VecStoreI32x4
            | IntrinsicOp::VecSadalpI32x4
            | IntrinsicOp::VecSmlalLoI32x4
            | IntrinsicOp::VecSmlalHiI32x4 => {
                // These are x86-64/AArch64-specific register-based vector operations
                unimplemented!("Register-based vector intrinsics not implemented for i686");
            }

            // ==================== AVX / AVX2 (VEX, 256-bit) ====================
            // Simple staging convention (i686 keeps vectors in stack slots —
            // the register allocator has no YMM class): load args through
            // %eax into ymm0/ymm1/ymm2, emit the VEX 3-op form, store %ymm0
            // back.  For non-commutative ops `op %ymm1, %ymm0, %ymm0`
            // computes args[0] OP args[1] (VEX.NDS: dst = vvvv OP r/m).
            IntrinsicOp::Paddb256
            | IntrinsicOp::Paddw256
            | IntrinsicOp::Paddd256
            | IntrinsicOp::Paddq256
            | IntrinsicOp::Psubb256
            | IntrinsicOp::Psubw256
            | IntrinsicOp::Psubd256
            | IntrinsicOp::Psubq256
            | IntrinsicOp::Psubusw256
            | IntrinsicOp::Pand256
            | IntrinsicOp::Por256
            | IntrinsicOp::Pxor256
            | IntrinsicOp::Pandn256
            | IntrinsicOp::Pmullw256
            | IntrinsicOp::Pmulhw256
            | IntrinsicOp::Pmuludq256
            | IntrinsicOp::Pmulld256
            | IntrinsicOp::Pmaddwd256
            | IntrinsicOp::Pmaddubsw256
            | IntrinsicOp::Pcmpeqb256
            | IntrinsicOp::Pcmpeqd256
            | IntrinsicOp::Pcmpeqq256
            | IntrinsicOp::Pcmpgtb256
            | IntrinsicOp::Pcmpgtd256
            | IntrinsicOp::Pcmpgtq256
            | IntrinsicOp::Pmaxub256
            | IntrinsicOp::Pmaxsd256
            | IntrinsicOp::Pminub256
            | IntrinsicOp::Pminsd256
            | IntrinsicOp::Psadbw256
            | IntrinsicOp::Pshufb256
            | IntrinsicOp::Packssdw256
            | IntrinsicOp::Packuswb256
            | IntrinsicOp::Phaddw256
            | IntrinsicOp::Phaddd256
            | IntrinsicOp::Punpcklbw256
            | IntrinsicOp::Punpckhbw256
            | IntrinsicOp::Punpcklwd256
            | IntrinsicOp::Punpckhwd256
            | IntrinsicOp::Punpckldq256
            | IntrinsicOp::Punpckhdq256
            | IntrinsicOp::Punpcklqdq256
            | IntrinsicOp::Punpckhqdq256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Paddb256 => "vpaddb",
                        IntrinsicOp::Paddw256 => "vpaddw",
                        IntrinsicOp::Paddd256 => "vpaddd",
                        IntrinsicOp::Paddq256 => "vpaddq",
                        IntrinsicOp::Psubb256 => "vpsubb",
                        IntrinsicOp::Psubw256 => "vpsubw",
                        IntrinsicOp::Psubd256 => "vpsubd",
                        IntrinsicOp::Psubq256 => "vpsubq",
                        IntrinsicOp::Psubusw256 => "vpsubusw",
                        IntrinsicOp::Pand256 => "vpand",
                        IntrinsicOp::Por256 => "vpor",
                        IntrinsicOp::Pxor256 => "vpxor",
                        IntrinsicOp::Pandn256 => "vpandn",
                        IntrinsicOp::Pmullw256 => "vpmullw",
                        IntrinsicOp::Pmulhw256 => "vpmulhw",
                        IntrinsicOp::Pmuludq256 => "vpmuludq",
                        IntrinsicOp::Pmulld256 => "vpmulld",
                        IntrinsicOp::Pmaddwd256 => "vpmaddwd",
                        IntrinsicOp::Pmaddubsw256 => "vpmaddubsw",
                        IntrinsicOp::Pcmpeqb256 => "vpcmpeqb",
                        IntrinsicOp::Pcmpeqd256 => "vpcmpeqd",
                        IntrinsicOp::Pcmpeqq256 => "vpcmpeqq",
                        IntrinsicOp::Pcmpgtb256 => "vpcmpgtb",
                        IntrinsicOp::Pcmpgtd256 => "vpcmpgtd",
                        IntrinsicOp::Pcmpgtq256 => "vpcmpgtq",
                        IntrinsicOp::Pmaxub256 => "vpmaxub",
                        IntrinsicOp::Pmaxsd256 => "vpmaxsd",
                        IntrinsicOp::Pminub256 => "vpminub",
                        IntrinsicOp::Pminsd256 => "vpminsd",
                        IntrinsicOp::Psadbw256 => "vpsadbw",
                        IntrinsicOp::Pshufb256 => "vpshufb",
                        IntrinsicOp::Packssdw256 => "vpackssdw",
                        IntrinsicOp::Packuswb256 => "vpackuswb",
                        IntrinsicOp::Phaddw256 => "vphaddw",
                        IntrinsicOp::Phaddd256 => "vphaddd",
                        IntrinsicOp::Punpcklbw256 => "vpunpcklbw",
                        IntrinsicOp::Punpckhbw256 => "vpunpckhbw",
                        IntrinsicOp::Punpcklwd256 => "vpunpcklwd",
                        IntrinsicOp::Punpckhwd256 => "vpunpckhwd",
                        IntrinsicOp::Punpckldq256 => "vpunpckldq",
                        IntrinsicOp::Punpckhdq256 => "vpunpckhdq",
                        IntrinsicOp::Punpcklqdq256 => "vpunpcklqdq",
                        IntrinsicOp::Punpckhqdq256 => "vpunpckhqdq",
                        _ => unreachable!("unexpected AVX2 integer binary op: {:?}", op),
                    };
                    self.emit_avx_binary_256(dptr, args, inst);
                }
            }

            // --- AVX packed float binaries (256-bit) ---
            IntrinsicOp::AddPs256
            | IntrinsicOp::SubPs256
            | IntrinsicOp::MulPs256
            | IntrinsicOp::AddPd256
            | IntrinsicOp::SubPd256
            | IntrinsicOp::MulPd256
            | IntrinsicOp::DivPs256
            | IntrinsicOp::DivPd256
            | IntrinsicOp::MinPs256
            | IntrinsicOp::MaxPs256
            | IntrinsicOp::MinPd256
            | IntrinsicOp::MaxPd256
            | IntrinsicOp::HaddPs256
            | IntrinsicOp::HsubPs256
            | IntrinsicOp::AddsubPs256
            | IntrinsicOp::UnpcklPs256
            | IntrinsicOp::UnpckhPs256
            | IntrinsicOp::UnpcklPd256
            | IntrinsicOp::UnpckhPd256
            | IntrinsicOp::VpermilvarPs256
            | IntrinsicOp::VpermilvarPd256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::AddPs256 => "vaddps",
                        IntrinsicOp::SubPs256 => "vsubps",
                        IntrinsicOp::MulPs256 => "vmulps",
                        IntrinsicOp::AddPd256 => "vaddpd",
                        IntrinsicOp::SubPd256 => "vsubpd",
                        IntrinsicOp::MulPd256 => "vmulpd",
                        IntrinsicOp::DivPs256 => "vdivps",
                        IntrinsicOp::DivPd256 => "vdivpd",
                        IntrinsicOp::MinPs256 => "vminps",
                        IntrinsicOp::MaxPs256 => "vmaxps",
                        IntrinsicOp::MinPd256 => "vminpd",
                        IntrinsicOp::MaxPd256 => "vmaxpd",
                        IntrinsicOp::HaddPs256 => "vhaddps",
                        IntrinsicOp::HsubPs256 => "vhsubps",
                        IntrinsicOp::AddsubPs256 => "vaddsubps",
                        IntrinsicOp::UnpcklPs256 => "vunpcklps",
                        IntrinsicOp::UnpckhPs256 => "vunpckhps",
                        IntrinsicOp::UnpcklPd256 => "vunpcklpd",
                        IntrinsicOp::UnpckhPd256 => "vunpckhpd",
                        IntrinsicOp::VpermilvarPs256 => "vpermilps",
                        IntrinsicOp::VpermilvarPd256 => "vpermilpd",
                        _ => unreachable!("unexpected AVX float binary op: {:?}", op),
                    };
                    self.emit_avx_binary_256(dptr, args, inst);
                }
            }

            // --- AVX packed float unary (256-bit) ---
            IntrinsicOp::SqrtPs256
            | IntrinsicOp::SqrtPd256
            | IntrinsicOp::CvtPs2Ep32_256
            | IntrinsicOp::CvttPs2Ep32_256
            | IntrinsicOp::CvtEp32ToPs256
            | IntrinsicOp::CvtPs2Pd256
            | IntrinsicOp::CvtPd2Ps256
            | IntrinsicOp::CvtPd2Ep32_256
            | IntrinsicOp::CvttPd2Ep32_256
            | IntrinsicOp::CvtEp32ToPd256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::SqrtPs256 => "vsqrtps",
                        IntrinsicOp::SqrtPd256 => "vsqrtpd",
                        IntrinsicOp::CvtPs2Ep32_256 => "vcvtps2dq",
                        IntrinsicOp::CvttPs2Ep32_256 => "vcvttps2dq",
                        IntrinsicOp::CvtEp32ToPs256 => "vcvtdq2ps",
                        IntrinsicOp::CvtPs2Pd256 => "vcvtps2pd",
                        IntrinsicOp::CvtPd2Ps256 => "vcvtpd2ps",
                        IntrinsicOp::CvtPd2Ep32_256 => "vcvtpd2dq",
                        IntrinsicOp::CvttPd2Ep32_256 => "vcvttpd2dq",
                        IntrinsicOp::CvtEp32ToPd256 => "vcvtdq2pd",
                        _ => unreachable!("unexpected AVX float unary op: {:?}", op),
                    };
                    self.emit_avx_load(&args[0], "ymm0");
                    self.state
                        .emit_fmt(format_args!("    {} %ymm0, %ymm0", inst));
                    self.emit_avx_store_dest(dptr);
                }
            }

            // 3-op with imm: vcmpps/cmppd/shufps/shufpd/blendps/blendpd (a, b, imm).
            IntrinsicOp::CmpPs256
            | IntrinsicOp::CmpPd256
            | IntrinsicOp::ShufPs256
            | IntrinsicOp::ShufPd256
            | IntrinsicOp::BlendPs256
            | IntrinsicOp::BlendPd256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::CmpPs256 => "vcmpps",
                        IntrinsicOp::CmpPd256 => "vcmppd",
                        IntrinsicOp::ShufPs256 => "vshufps",
                        IntrinsicOp::ShufPd256 => "vshufpd",
                        IntrinsicOp::BlendPs256 => "vblendps",
                        IntrinsicOp::BlendPd256 => "vblendpd",
                        _ => unreachable!("unexpected AVX fp imm op: {:?}", op),
                    };
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.emit_avx_load(&args[0], "ymm0");
                    self.emit_avx_load(&args[1], "ymm1");
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %ymm1, %ymm0, %ymm0", inst, imm));
                    self.emit_avx_store_dest(dptr);
                }
            }

            // 2-op with imm: vroundps/vroundpd/vpermilps imm (a, imm).
            IntrinsicOp::RoundPs256 | IntrinsicOp::RoundPd256 | IntrinsicOp::VpermilPs256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::RoundPs256 => "vroundps",
                        IntrinsicOp::RoundPd256 => "vroundpd",
                        IntrinsicOp::VpermilPs256 => "vpermilps",
                        _ => unreachable!("unexpected AVX round op: {:?}", op),
                    };
                    let imm = Self::operand_to_imm_i64(&args[1]);
                    self.emit_avx_load(&args[0], "ymm0");
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %ymm0, %ymm0", inst, imm));
                    self.emit_avx_store_dest(dptr);
                }
            }

            // vperm2f128 (a, b, imm).
            IntrinsicOp::Vperm2f128 | IntrinsicOp::Permute2x128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Vperm2f128) {
                        "vperm2f128"
                    } else {
                        "vperm2i128"
                    };
                    let imm = Self::operand_to_imm_i64(&args[2]);
                    self.emit_avx_load(&args[0], "ymm0");
                    self.emit_avx_load(&args[1], "ymm1");
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %ymm1, %ymm0, %ymm0", inst, imm));
                    self.emit_avx_store_dest(dptr);
                }
            }

            // vinsertf128/vinserti128 (a, b128, imm).
            IntrinsicOp::Vinsertf128 | IntrinsicOp::Insert128to256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Vinsertf128) {
                        "vinsertf128"
                    } else {
                        "vinserti128"
                    };
                    let imm = Self::operand_to_imm_i64(&args[2]) & 1;
                    self.emit_avx_load(&args[0], "ymm0");
                    self.emit_avx_load128(&args[1], "xmm1");
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %xmm1, %ymm0, %ymm0", inst, imm));
                    self.emit_avx_store_dest(dptr);
                }
            }

            // vextractf128/vextracti128 (a, imm) -> 128-bit dest.
            IntrinsicOp::Vextractf128 | IntrinsicOp::Extracti128 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Vextractf128) {
                        "vextractf128"
                    } else {
                        "vextracti128"
                    };
                    let imm = Self::operand_to_imm_i64(&args[1]) & 1;
                    self.emit_avx_load(&args[0], "ymm0");
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %ymm0, %xmm0", inst, imm));
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    vmovdqu %xmm0, (%eax)");
                }
            }

            // vbroadcastss/vbroadcastsd (128-bit src -> ymm).
            IntrinsicOp::Vbroadcastss | IntrinsicOp::Vbroadcastsd => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::Vbroadcastss) {
                        "vbroadcastss"
                    } else {
                        "vbroadcastsd"
                    };
                    self.emit_avx_load128(&args[0], "xmm1");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm1, %ymm0", inst));
                    self.emit_avx_store_dest(dptr);
                }
            }

            // Broadcast a 128-bit lane pair (vbroadcasti128 semantics):
            // vinserti128 $1 duplicates the low lane into the high lane.
            IntrinsicOp::Broadcast128to256 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_avx_load128(&args[0], "xmm0");
                    self.state.emit("    vinserti128 $1, %xmm0, %ymm0, %ymm0");
                    self.emit_avx_store_dest(dptr);
                }
            }

            // Zero-extend 128 -> 256 (high lane zero, NOT a duplicate).
            IntrinsicOp::Zext128to256 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_avx_load128(&args[0], "xmm0");
                    self.state.emit("    vpxor %ymm1, %ymm1, %ymm1");
                    self.state.emit("    vinserti128 $0, %xmm0, %ymm1, %ymm0");
                    self.emit_avx_store_dest(dptr);
                }
            }

            // Truncate 256 -> 128 (low lane only).
            IntrinsicOp::Cast256to128 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_avx_load(&args[0], "ymm0");
                    self.state.emit("    vextracti128 $0, %ymm0, %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    vmovdqu %xmm0, (%eax)");
                }
            }

            // vblendvps/vblendvpd: operands (mask, a, b); result must land
            // in %ymm0. AT&T: %mask(is4), %src2, %src1(dst), %dst.
            IntrinsicOp::BlendvPs256 | IntrinsicOp::BlendvPd256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = if matches!(op, IntrinsicOp::BlendvPs256) {
                        "vblendvps"
                    } else {
                        "vblendvpd"
                    };
                    self.emit_avx_load(&args[0], "ymm2"); // mask
                    self.emit_avx_load(&args[1], "ymm0"); // a (src1 = dst)
                    self.emit_avx_load(&args[2], "ymm1"); // b (src2)
                    self.state
                        .emit_fmt(format_args!("    {} %ymm2, %ymm1, %ymm0, %ymm0", inst));
                    self.emit_avx_store_dest(dptr);
                }
            }

            // FMA 256 (a, b, c): vfmadd*ps/pd %ymm1, %ymm2, %ymm0.
            IntrinsicOp::FmaPs132v256
            | IntrinsicOp::FmaPs213v256
            | IntrinsicOp::FmaPs231v256
            | IntrinsicOp::FmaPd132v256
            | IntrinsicOp::FmaPd213v256
            | IntrinsicOp::FmaPd231v256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::FmaPs132v256 => "vfmadd132ps",
                        IntrinsicOp::FmaPs213v256 => "vfmadd213ps",
                        IntrinsicOp::FmaPs231v256 => "vfmadd231ps",
                        IntrinsicOp::FmaPd132v256 => "vfmadd132pd",
                        IntrinsicOp::FmaPd213v256 => "vfmadd213pd",
                        IntrinsicOp::FmaPd231v256 => "vfmadd231pd",
                        _ => unreachable!("unexpected FMA op: {:?}", op),
                    };
                    self.emit_avx_load(&args[0], "ymm0");
                    self.emit_avx_load(&args[1], "ymm1");
                    self.emit_avx_load(&args[2], "ymm2");
                    self.state
                        .emit_fmt(format_args!("    {} %ymm1, %ymm2, %ymm0", inst));
                    self.emit_avx_store_dest(dptr);
                }
            }

            // VPERMD: dest[i] = src[idx[i] & 7]. AT&T: vpermd %src, %idx, %dst.
            IntrinsicOp::Permutevar8x32 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_avx_load(&args[0], "ymm0");
                    self.emit_avx_load(&args[1], "ymm1");
                    self.state.emit("    vpermd %ymm0, %ymm1, %ymm0");
                    self.emit_avx_store_dest(dptr);
                }
            }

            IntrinsicOp::Pshufd256 => {
                if let Some(dptr) = dest_ptr {
                    let imm = Self::operand_to_imm_i64(&args[1]);
                    self.emit_avx_load(&args[0], "ymm0");
                    self.state
                        .emit_fmt(format_args!("    vpshufd ${}, %ymm0, %ymm0", imm));
                    self.emit_avx_store_dest(dptr);
                }
            }

            IntrinsicOp::Permute4x64 => {
                if let Some(dptr) = dest_ptr {
                    let imm = Self::operand_to_imm_i64(&args[1]);
                    self.emit_avx_load(&args[0], "ymm0");
                    self.state
                        .emit_fmt(format_args!("    vpermq ${}, %ymm0, %ymm0", imm));
                    self.emit_avx_store_dest(dptr);
                }
            }

            // 256-bit byte-lane shifts by immediate.
            IntrinsicOp::Pslldqi256 | IntrinsicOp::Psrldqi256 => {
                if let Some(dptr) = dest_ptr {
                    let imm = Self::operand_to_imm_i64(&args[1]) & 0xff;
                    let inst = if matches!(op, IntrinsicOp::Pslldqi256) {
                        "vpslldq"
                    } else {
                        "vpsrldq"
                    };
                    self.emit_avx_load(&args[0], "ymm0");
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %ymm0, %ymm0", inst, imm));
                    self.emit_avx_store_dest(dptr);
                }
            }

            // 256-bit element shifts by immediate.
            IntrinsicOp::Psllqi256
            | IntrinsicOp::Psrlqi256
            | IntrinsicOp::Psllidi256
            | IntrinsicOp::Psrlidi256
            | IntrinsicOp::Psllwi256
            | IntrinsicOp::Psrlwi256
            | IntrinsicOp::Psrawi256
            | IntrinsicOp::Psradi256 => {
                if let Some(dptr) = dest_ptr {
                    let imm = Self::operand_to_imm_i64(&args[1]);
                    let inst = match op {
                        IntrinsicOp::Psllqi256 => "vpsllq",
                        IntrinsicOp::Psrlqi256 => "vpsrlq",
                        IntrinsicOp::Psllidi256 => "vpslld",
                        IntrinsicOp::Psrlidi256 => "vpsrld",
                        IntrinsicOp::Psllwi256 => "vpsllw",
                        IntrinsicOp::Psrlwi256 => "vpsrlw",
                        IntrinsicOp::Psrawi256 => "vpsraw",
                        IntrinsicOp::Psradi256 => "vpsrad",
                        _ => unreachable!("unexpected AVX shift op: {:?}", op),
                    };
                    self.emit_avx_load(&args[0], "ymm0");
                    self.state
                        .emit_fmt(format_args!("    {} ${}, %ymm0, %ymm0", inst, imm));
                    self.emit_avx_store_dest(dptr);
                }
            }

            // SSE4.1 widening conversions with a 128-bit source, 256-bit dest.
            IntrinsicOp::Pmovzxbw256
            | IntrinsicOp::Pmovzxbd256
            | IntrinsicOp::Pmovzxwd256
            | IntrinsicOp::Pmovsxbw256
            | IntrinsicOp::Pmovsxbd256
            | IntrinsicOp::Pmovsxwd256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Pmovzxbw256 => "vpmovzxbw",
                        IntrinsicOp::Pmovzxbd256 => "vpmovzxbd",
                        IntrinsicOp::Pmovzxwd256 => "vpmovzxwd",
                        IntrinsicOp::Pmovsxbw256 => "vpmovsxbw",
                        IntrinsicOp::Pmovsxbd256 => "vpmovsxbd",
                        IntrinsicOp::Pmovsxwd256 => "vpmovsxwd",
                        _ => unreachable!("unexpected AVX widening op: {:?}", op),
                    };
                    self.emit_avx_load128(&args[0], "xmm0");
                    self.state
                        .emit_fmt(format_args!("    {} %xmm0, %ymm0", inst));
                    self.emit_avx_store_dest(dptr);
                }
            }

            // 256-bit absolute value (unary).
            IntrinsicOp::Pabsb256 | IntrinsicOp::Pabsw256 | IntrinsicOp::Pabsd256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Pabsb256 => "vpabsb",
                        IntrinsicOp::Pabsw256 => "vpabsw",
                        IntrinsicOp::Pabsd256 => "vpabsd",
                        _ => unreachable!("unexpected AVX abs op: {:?}", op),
                    };
                    self.emit_avx_load(&args[0], "ymm0");
                    self.state
                        .emit_fmt(format_args!("    {} %ymm0, %ymm0", inst));
                    self.emit_avx_store_dest(dptr);
                }
            }

            // Runtime splats: vmovd the scalar into %xmm0, then broadcast.
            // (i686 has no rip-relative vector-constant pool; the x86-64
            // backend's .rodata splat path does not port directly.)
            IntrinsicOp::SetEpi8_256
            | IntrinsicOp::SetEpi16_256
            | IntrinsicOp::SetEpi32_256
            | IntrinsicOp::SetEpi64x256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::SetEpi8_256 => "vpbroadcastb",
                        IntrinsicOp::SetEpi16_256 => "vpbroadcastw",
                        IntrinsicOp::SetEpi32_256 => "vpbroadcastd",
                        IntrinsicOp::SetEpi64x256 => "vpbroadcastq",
                        _ => unreachable!("unexpected AVX splat op: {:?}", op),
                    };
                    if matches!(op, IntrinsicOp::SetEpi64x256) {
                        // 64-bit element: stage BOTH halves (push hi, push lo
                        // => (%esp) holds the little-endian qword), vmovsd into
                        // %xmm0, then broadcast.
                        match &args[0] {
                            Operand::Value(v) => {
                                if let Some(slot) = self.state.get_slot(v.0) {
                                    let sr0 = self.slot_ref(slot);
                                    let sr4 = self.slot_ref_offset(slot, 4);
                                    emit!(self.state, "    movl {}, %eax", sr4);
                                    self.state.emit("    pushl %eax");
                                    emit!(self.state, "    movl {}, %eax", sr0);
                                    self.state.emit("    pushl %eax");
                                } else {
                                    self.operand_to_eax(&args[0]);
                                    self.state.emit("    pushl %eax");
                                    self.state.emit("    pushl $0");
                                }
                            }
                            Operand::Const(IrConst::I64(v)) => {
                                let bits = *v as u64;
                                emit!(self.state, "    pushl ${}", ((bits >> 32) as u32) as i32);
                                emit!(self.state, "    pushl ${}", (bits as u32) as i32);
                            }
                            _ => {
                                self.operand_to_eax(&args[0]);
                                self.state.emit("    pushl %eax");
                                self.state.emit("    pushl $0");
                            }
                        }
                        self.state.emit("    vmovsd (%esp), %xmm0");
                        self.state.emit("    vpbroadcastq %xmm0, %ymm0");
                        self.state.emit("    addl $8, %esp");
                    } else {
                        self.operand_to_eax(&args[0]);
                        self.state.emit("    vmovd %eax, %xmm0");
                        self.state
                            .emit_fmt(format_args!("    {} %xmm0, %ymm0", inst));
                    }
                    self.emit_avx_store_dest(dptr);
                }
            }

            IntrinsicOp::Loadu256 | IntrinsicOp::Load256 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_avx_load(&args[0], "ymm0");
                    self.emit_avx_store_dest(dptr);
                }
            }
            IntrinsicOp::Storeu256 | IntrinsicOp::Store256 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_avx_load(&args[0], "ymm0");
                    self.emit_avx_store_dest(dptr);
                }
            }
            IntrinsicOp::LoaduPs256 | IntrinsicOp::LoaduPd256 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_avx_load(&args[0], "ymm0");
                    self.emit_avx_store_dest(dptr);
                }
            }
            IntrinsicOp::StoreuPs256 | IntrinsicOp::StoreuPd256 => {
                if let Some(dptr) = dest_ptr {
                    self.emit_avx_load(&args[0], "ymm0");
                    self.emit_avx_store_dest(dptr);
                }
            }

            // GPR-result 256-bit ops.
            IntrinsicOp::Pmovmskb256
            | IntrinsicOp::MovemaskPs256
            | IntrinsicOp::MovemaskPd256 => {
                let inst = match op {
                    IntrinsicOp::Pmovmskb256 => "vpmovmskb",
                    IntrinsicOp::MovemaskPs256 => "vmovmskps",
                    _ => "vmovmskpd",
                };
                self.emit_avx_load(&args[0], "ymm0");
                self.state
                    .emit_fmt(format_args!("    {} %ymm0, %eax", inst));
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }

            // VTESTPS/PD: ZF semantics identical to ptest.
            IntrinsicOp::TestzPs256 => {
                self.emit_avx_load(&args[0], "ymm0");
                self.emit_avx_load(&args[1], "ymm1");
                self.state.emit("    vtestps %ymm1, %ymm0");
                self.state.emit("    sete %al");
                self.state.emit("    movzbl %al, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
                }
            }

            IntrinsicOp::Vzeroupper => {
                self.state.emit("    vzeroupper");
            }

            IntrinsicOp::Setzero256 => {
                if let Some(dptr) = dest_ptr {
                    self.state.emit("    vpxor %ymm0, %ymm0, %ymm0");
                    self.emit_avx_store_dest(dptr);
                }
            }

            // AVX-VNNI dot products (a, b, c) — 66/F2/F3/NP 0F38 pp forms.
            IntrinsicOp::Dpbusd128
            | IntrinsicOp::Dpbusds128
            | IntrinsicOp::Dpwusd128
            | IntrinsicOp::Dpwusds128
            | IntrinsicOp::Dpbssd128
            | IntrinsicOp::Dpbssds128
            | IntrinsicOp::Dpbsud128
            | IntrinsicOp::Dpbsuds128
            | IntrinsicOp::Dpbuud128
            | IntrinsicOp::Dpbuuds128
            | IntrinsicOp::Dpwuud128
            | IntrinsicOp::Dpwuuds128
            | IntrinsicOp::Dpwssd128
            | IntrinsicOp::Dpwssds128
            | IntrinsicOp::Dpbusd256
            | IntrinsicOp::Dpbusds256
            | IntrinsicOp::Dpwusd256
            | IntrinsicOp::Dpwusds256
            | IntrinsicOp::Dpbssd256
            | IntrinsicOp::Dpbssds256
            | IntrinsicOp::Dpbsud256
            | IntrinsicOp::Dpbsuds256
            | IntrinsicOp::Dpbuud256
            | IntrinsicOp::Dpbuuds256
            | IntrinsicOp::Dpwuud256
            | IntrinsicOp::Dpwuuds256
            | IntrinsicOp::Dpwssd256
            | IntrinsicOp::Dpwssds256 => {
                if let Some(dptr) = dest_ptr {
                    let inst = match op {
                        IntrinsicOp::Dpbusd128 | IntrinsicOp::Dpbusd256 => "vpdpbusd",
                        IntrinsicOp::Dpbusds128 | IntrinsicOp::Dpbusds256 => "vpdpbusds",
                        IntrinsicOp::Dpwusd128 | IntrinsicOp::Dpwusd256 => "vpdpwusd",
                        IntrinsicOp::Dpwusds128 | IntrinsicOp::Dpwusds256 => "vpdpwusds",
                        IntrinsicOp::Dpbssd128 | IntrinsicOp::Dpbssd256 => "vpdpbssd",
                        IntrinsicOp::Dpbssds128 | IntrinsicOp::Dpbssds256 => "vpdpbssds",
                        IntrinsicOp::Dpbsud128 | IntrinsicOp::Dpbsud256 => "vpdpbsud",
                        IntrinsicOp::Dpbsuds128 | IntrinsicOp::Dpbsuds256 => "vpdpbsuds",
                        IntrinsicOp::Dpbuud128 | IntrinsicOp::Dpbuud256 => "vpdpbuud",
                        IntrinsicOp::Dpbuuds128 | IntrinsicOp::Dpbuuds256 => "vpdpbuuds",
                        IntrinsicOp::Dpwuud128 | IntrinsicOp::Dpwuud256 => "vpdpwuud",
                        IntrinsicOp::Dpwuuds128 | IntrinsicOp::Dpwuuds256 => "vpdpwuuds",
                        IntrinsicOp::Dpwssd128 | IntrinsicOp::Dpwssd256 => "vpdpwssd",
                        IntrinsicOp::Dpwssds128 | IntrinsicOp::Dpwssds256 => "vpdpwssds",
                        _ => unreachable!("unexpected VNNI op: {:?}", op),
                    };
                    let is256 = matches!(
                        op,
                        IntrinsicOp::Dpbusd256
                            | IntrinsicOp::Dpbusds256
                            | IntrinsicOp::Dpwusd256
                            | IntrinsicOp::Dpwusds256
                            | IntrinsicOp::Dpbssd256
                            | IntrinsicOp::Dpbssds256
                            | IntrinsicOp::Dpbsud256
                            | IntrinsicOp::Dpbsuds256
                            | IntrinsicOp::Dpbuud256
                            | IntrinsicOp::Dpbuuds256
                            | IntrinsicOp::Dpwuud256
                            | IntrinsicOp::Dpwuuds256
                            | IntrinsicOp::Dpwssd256
                            | IntrinsicOp::Dpwssds256
                    );
                    if is256 {
                        self.emit_avx_load(&args[0], "ymm0");
                        self.emit_avx_load(&args[1], "ymm1");
                        self.emit_avx_load(&args[2], "ymm2");
                        self.state
                            .emit_fmt(format_args!("    {} %ymm2, %ymm1, %ymm0", inst));
                        self.emit_avx_store_dest(dptr);
                    } else {
                        self.operand_to_eax(&args[0]);
                        self.state.emit("    movdqu (%eax), %xmm0");
                        self.operand_to_eax(&args[1]);
                        self.state.emit("    movdqu (%eax), %xmm1");
                        self.operand_to_eax(&args[2]);
                        self.state.emit("    movdqu (%eax), %xmm2");
                        self.state
                            .emit_fmt(format_args!("    {} %xmm2, %xmm1, %xmm0", inst));
                        self.operand_to_eax(&Operand::Value(*dptr));
                        self.state.emit("    vmovdqu %xmm0, (%eax)");
                    }
                }
            }

            _ => { /* x86-only SIMD op on i686: no-op */ }
        }
        self.state.reg_cache.invalidate_acc();
    }

    fn emit_nontemporal_store(
        &mut self,
        op: &IntrinsicOp,
        dest_ptr: &Option<Value>,
        args: &[Operand],
    ) {
        let Some(ptr) = dest_ptr else { return };
        match op {
            IntrinsicOp::Movnti => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    movl %eax, %ecx");
                self.operand_to_eax(&Operand::Value(*ptr));
                self.state.emit("    movnti %ecx, (%eax)");
            }
            IntrinsicOp::Movnti64 => {
                self.operand_to_eax(&Operand::Value(*ptr));
                self.state.emit("    movl %eax, %ecx");
                if let Operand::Value(v) = &args[0] {
                    if let Some(slot) = self.state.get_slot(v.0) {
                        let sr0 = self.slot_ref(slot);
                        let sr4 = self.slot_ref_offset(slot, 4);
                        emit!(self.state, "    movl {}, %eax", sr0);
                        self.state.emit("    movnti %eax, (%ecx)");
                        emit!(self.state, "    movl {}, %eax", sr4);
                        self.state.emit("    movnti %eax, 4(%ecx)");
                    } else {
                        self.operand_to_eax(&args[0]);
                        self.state.emit("    movnti %eax, (%ecx)");
                        self.state.emit("    xorl %eax, %eax");
                        self.state.emit("    movnti %eax, 4(%ecx)");
                    }
                } else {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movnti %eax, (%ecx)");
                    self.state.emit("    xorl %eax, %eax");
                    self.state.emit("    movnti %eax, 4(%ecx)");
                }
            }
            IntrinsicOp::Movntdq => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.operand_to_eax(&Operand::Value(*ptr));
                self.state.emit("    movntdq %xmm0, (%eax)");
            }
            IntrinsicOp::Movntpd => {
                self.operand_to_eax(&args[0]);
                self.state.emit("    movupd (%eax), %xmm0");
                self.operand_to_eax(&Operand::Value(*ptr));
                self.state.emit("    movntpd %xmm0, (%eax)");
            }
            _ => {}
        }
    }

    fn emit_crc32_intrinsic(&mut self, op: &IntrinsicOp, dest: &Option<Value>, args: &[Operand]) {
        if *op == IntrinsicOp::Crc32_64 {
            // On i686, no 64-bit CRC32; do two 32-bit CRC32s
            self.operand_to_eax(&args[0]);
            self.state.emit("    movl %eax, %edx");
            if let Operand::Value(v) = &args[1] {
                if let Some(slot) = self.state.get_slot(v.0) {
                    let sr0 = self.slot_ref(slot);
                    let sr4 = self.slot_ref_offset(slot, 4);
                    emit!(self.state, "    movl {}, %ecx", sr0);
                    self.state.emit("    movl %edx, %eax");
                    self.state.emit("    crc32l %ecx, %eax");
                    emit!(self.state, "    movl {}, %ecx", sr4);
                    self.state.emit("    crc32l %ecx, %eax");
                } else {
                    self.operand_to_ecx(&args[1]);
                    self.state.emit("    movl %edx, %eax");
                    self.state.emit("    crc32l %ecx, %eax");
                }
            } else {
                self.operand_to_ecx(&args[1]);
                self.state.emit("    movl %edx, %eax");
                self.state.emit("    crc32l %ecx, %eax");
            }
        } else {
            self.operand_to_eax(&args[0]);
            self.state.emit("    movl %eax, %ecx");
            self.operand_to_eax(&args[1]);
            self.state.emit("    xchgl %eax, %ecx");
            let inst = match op {
                IntrinsicOp::Crc32_8 => "crc32b %cl, %eax",
                IntrinsicOp::Crc32_16 => "crc32w %cx, %eax",
                IntrinsicOp::Crc32_32 => "crc32l %ecx, %eax",
                _ => unreachable!("unexpected CRC32 op: {:?}", op),
            };
            self.state.emit_fmt(format_args!("    {}", inst));
        }
        self.state.reg_cache.invalidate_acc();
        if let Some(d) = dest {
            self.store_eax_to(d);
        }
    }

    /// Apply an x87 unary FPU op on an f64 operand and store the result.
    fn emit_f64_unary_x87(&mut self, arg: &Operand, x87_op: &str, dest: &Option<Value>) {
        self.emit_f64_load_to_x87(arg);
        self.state.emit_fmt(format_args!("    {}", x87_op));
        if let Some(d) = dest {
            if let Some(slot) = self.state.get_slot(d.0) {
                let sr = self.slot_ref(slot);
                emit!(self.state, "    fstpl {}", sr);
            } else {
                self.state.emit("    fstp %st(0)");
            }
        } else {
            self.state.emit("    fstp %st(0)");
        }
    }

    /// S11: `frndint` under the rounding-control mode demanded by a
    /// RoundScalar immediate. Encoding map (matches simplify.rs's
    /// GCC-verified roundsd immediates): 8 = roundeven (RC 00 nearest-even),
    /// 9 = floor (RC 01 down), 10 = ceil (RC 10 up), 11 = trunc (RC 11
    /// chop), 4 = rint / 12 = nearbyint (ambient mode, no CW touch).
    ///
    /// RC switching protocol: save the CW into the dynamic 4-byte scratch,
    /// keep the ORIGINAL in %ax, write the modified word back over the
    /// scratch, `fldcw`, `frndint`, then restore the original word and
    /// `fldcw` again — the FPU control word is per-thread state that must
    /// be restored even for a single-instruction window (a caller may hold
    /// a non-default mode, e.g. Fortran/MPFR-style code). %eax/%edx are
    /// free here: the operand was already staged onto the x87 stack by the
    /// caller's load helper, so no GPR holds live data across this window.
    fn emit_x87_frndint_with_mode(&mut self, imm: u8) {
        match imm {
            8 | 9 | 10 | 11 => {
                let rc: u16 = match imm {
                    8 => 0x0000, // nearest-even
                    9 => 0x0400, // down (floor)
                    10 => 0x0800, // up (ceil)
                    _ => 0x0c00, // chop (trunc)
                };
                self.state.emit("    subl $4, %esp");
                self.state.emit("    fnstcw (%esp)");
                self.state.emit("    movw (%esp), %ax");
                self.state.emit("    movw %ax, %dx");
                self.state.emit("    andw $0xf3ff, %dx");
                self.state
                    .emit_fmt(format_args!("    orw ${}, %dx", rc));
                self.state.emit("    movw %dx, (%esp)");
                self.state.emit("    fldcw (%esp)");
                self.state.emit("    frndint");
                self.state.emit("    movw %ax, (%esp)");
                self.state.emit("    fldcw (%esp)");
                self.state.emit("    addl $4, %esp");
            }
            _ => {
                // rint (4) / nearbyint (12): round in the ambient mode.
                // Any unknown immediate also lands here: the ambient-mode
                // frndint is the safe conservative behavior, never a
                // dropped instruction.
                self.state.emit("    frndint");
            }
        }
    }

    /// Stage a scalar float VALUE's bit pattern into an XMM register.
    /// F32: slot-resident values load with movss; constants push their bits.
    /// F64 values are 8-byte slot pairs loaded with movsd; F32 goes through
    /// the GPR bit-pattern staging the i686 backend uses everywhere.
    fn emit_f32_scalar_bits_to_xmm(&mut self, arg: &Operand, xmm: &str) {
        match arg {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    let sr = self.slot_ref(slot);
                    emit!(self.state, "    movss {}, %{}", sr, xmm);
                    return;
                }
                self.operand_to_eax(arg);
                self.state
                    .emit_fmt(format_args!("    movd %eax, %{}", xmm));
            }
            Operand::Const(IrConst::F32(f)) => {
                emit!(self.state, "    pushl ${}", f.to_bits() as i32);
                self.state
                    .emit_fmt(format_args!("    movss (%esp), %{}", xmm));
                self.state.emit("    addl $4, %esp");
            }
            Operand::Const(IrConst::F64(f)) => {
                let bits = f.to_bits();
                emit!(self.state, "    pushl ${}", ((bits >> 32) as u32) as i32);
                emit!(self.state, "    pushl ${}", (bits as u32) as i32);
                self.state
                    .emit_fmt(format_args!("    movsd (%esp), %{}", xmm));
                self.state.emit("    addl $8, %esp");
            }
            _ => {
                self.operand_to_eax(arg);
                self.state
                    .emit_fmt(format_args!("    movd %eax, %{}", xmm));
            }
        }
    }

    /// Stage a scalar F64 VALUE's bit pattern into an XMM register (movsd from
    /// the 8-byte slot pair; constants push hi/lo and load from (%esp)).
    fn emit_f64_scalar_bits_to_xmm(&mut self, arg: &Operand, xmm: &str) {
        match arg {
            Operand::Value(v) => {
                if let Some(slot) = self.state.get_slot(v.0) {
                    let sr = self.slot_ref(slot);
                    emit!(self.state, "    movsd {}, %{}", sr, xmm);
                    return;
                }
                self.operand_to_eax(arg);
                self.state
                    .emit_fmt(format_args!("    movd %eax, %{}", xmm));
            }
            Operand::Const(IrConst::F64(f)) => {
                let bits = f.to_bits();
                emit!(self.state, "    pushl ${}", ((bits >> 32) as u32) as i32);
                emit!(self.state, "    pushl ${}", (bits as u32) as i32);
                self.state
                    .emit_fmt(format_args!("    movsd (%esp), %{}", xmm));
                self.state.emit("    addl $8, %esp");
            }
            Operand::Const(IrConst::F32(f)) => {
                emit!(self.state, "    pushl ${}", f.to_bits() as i32);
                self.state
                    .emit_fmt(format_args!("    movss (%esp), %{}", xmm));
                self.state.emit("    addl $4, %esp");
            }
            _ => {
                self.operand_to_eax(arg);
                self.state
                    .emit_fmt(format_args!("    movd %eax, %{}", xmm));
            }
        }
    }

    /// Emit a binary SSE 128-bit operation: load two 128-bit operands from
    /// pointers, apply the operation, and store the result to dest_ptr.
    fn emit_sse_binary_128(&mut self, dptr: &Value, args: &[Operand], sse_inst: &str) {
        self.operand_to_eax(&args[0]);
        self.state.emit("    movdqu (%eax), %xmm0");
        self.operand_to_eax(&args[1]);
        self.state.emit("    movdqu (%eax), %xmm1");
        self.state
            .emit_fmt(format_args!("    {} %xmm1, %xmm0", sse_inst));
        self.operand_to_eax(&Operand::Value(*dptr));
        self.state.emit("    movdqu %xmm0, (%eax)");
    }

    /// Load a 256-bit operand (an alloca pointer) into a YMM register via
    /// %eax.  Slots are only 8-aligned, so the unaligned vmovdqu form is used.
    fn emit_avx_load(&mut self, arg: &Operand, ymm: &str) {
        self.operand_to_eax(arg);
        self.state
            .emit_fmt(format_args!("    vmovdqu (%eax), %{}", ymm));
    }

    /// Load a 128-bit operand (an alloca pointer) into an XMM register via
    /// %eax — used by the 128↔256 bridging ops.
    fn emit_avx_load128(&mut self, arg: &Operand, xmm: &str) {
        self.operand_to_eax(arg);
        self.state
            .emit_fmt(format_args!("    vmovdqu (%eax), %{}", xmm));
    }

    /// Store %ymm0 to a 256-bit destination's slot and invalidate the
    /// accumulator cache (the address staging used %eax).
    fn emit_avx_store_dest(&mut self, dptr: &Value) {
        self.operand_to_eax(&Operand::Value(*dptr));
        self.state.emit("    vmovdqu %ymm0, (%eax)");
        self.state.reg_cache.invalidate_acc();
    }

    /// Emit a binary AVX/AVX2 256-bit operation with the simple staging
    /// convention: a → %ymm0, b → %ymm1, `inst %ymm1, %ymm0, %ymm0`
    /// (VEX.NDS: dst = vvvv OP r/m, i.e. args[0] OP args[1] for both
    /// commutative and non-commutative mnemonics), result → dest slot.
    fn emit_avx_binary_256(&mut self, dptr: &Value, args: &[Operand], inst: &str) {
        self.emit_avx_load(&args[0], "ymm0");
        self.emit_avx_load(&args[1], "ymm1");
        self.state
            .emit_fmt(format_args!("    {} %ymm1, %ymm0, %ymm0", inst));
        self.emit_avx_store_dest(dptr);
    }

    /// Emit SSE unary 128-bit op with immediate: load xmm0 from arg0 ptr,
    /// apply `inst $imm, %xmm0`, store result xmm0 to dest_ptr.
    fn emit_sse_unary_imm_128(&mut self, dptr: &Value, args: &[Operand], sse_inst: &str) {
        self.operand_to_eax(&args[0]);
        self.state.emit("    movdqu (%eax), %xmm0");
        let imm = Self::operand_to_imm_i64(&args[1]);
        self.state
            .emit_fmt(format_args!("    {} ${}, %xmm0", sse_inst, imm));
        self.operand_to_eax(&Operand::Value(*dptr));
        self.state.emit("    movdqu %xmm0, (%eax)");
    }

    /// Emit SSE shuffle with immediate: load xmm0, apply `inst $imm, %xmm0, %xmm0`,
    /// store result. Used for pshufd/pshuflw/pshufhw.
    fn emit_sse_shuffle_imm_128(&mut self, dptr: &Value, args: &[Operand], sse_inst: &str) {
        self.operand_to_eax(&args[0]);
        self.state.emit("    movdqu (%eax), %xmm0");
        let imm = Self::operand_to_imm_i64(&args[1]);
        self.state
            .emit_fmt(format_args!("    {} ${}, %xmm0, %xmm0", sse_inst, imm));
        self.operand_to_eax(&Operand::Value(*dptr));
        self.state.emit("    movdqu %xmm0, (%eax)");
    }

    /// Load an F32 operand onto the x87 FPU stack.
    fn emit_f32_load_to_x87(&mut self, op: &Operand) {
        match op {
            Operand::Value(v) if self.state.get_slot(v.0).is_some() => {
                let slot = self
                    .state
                    .get_slot(v.0)
                    .expect("slot exists (guarded by is_some)");
                let sr = self.slot_ref(slot);
                emit!(self.state, "    flds {}", sr);
            }
            Operand::Const(IrConst::F32(fval)) => {
                emit!(self.state, "    movl ${}, %eax", fval.to_bits() as i32);
                self.state.emit("    pushl %eax");
                self.state.emit("    flds (%esp)");
                self.state.emit("    addl $4, %esp");
            }
            _ => {
                self.operand_to_eax(op);
                self.state.emit("    pushl %eax");
                self.state.emit("    flds (%esp)");
                self.state.emit("    addl $4, %esp");
            }
        }
    }

    /// Store an x87 FPU result as F32 to a destination value.
    fn emit_f32_store_from_x87(&mut self, dest: &Option<Value>) {
        if let Some(d) = dest {
            if let Some(slot) = self.state.get_slot(d.0) {
                let sr = self.slot_ref(slot);
                emit!(self.state, "    fstps {}", sr);
            } else {
                self.state.emit("    fstp %st(0)");
            }
        } else {
            self.state.emit("    fstp %st(0)");
        }
    }

    /// Emit copysign for 64-bit float (F64): magnitude of x, sign of y.
    fn emit_f64_copysign(&mut self, dest: &Option<Value>, x: &Operand, y: &Operand) {
        let Some(d) = dest else { return };
        let Some(dest_slot) = self.state.get_slot(d.0) else { return };
        let dsr0 = self.slot_ref(dest_slot);
        let dsr4 = self.slot_ref_offset(dest_slot, 4);

        // Copy lower 32 bits of x directly to dest_slot low word
        match x {
            Operand::Const(IrConst::F64(f)) => {
                let lo = f.to_bits() as u32 as i32;
                emit!(self.state, "    movl ${}, {}", lo, dsr0);
            }
            Operand::Value(v) if self.state.get_slot(v.0).is_some() => {
                let x_slot = self.state.get_slot(v.0).unwrap();
                let sr0 = self.slot_ref(x_slot);
                emit!(self.state, "    movl {}, %eax", sr0);
                emit!(self.state, "    movl %eax, {}", dsr0);
            }
            _ => {
                self.operand_to_eax(x);
                emit!(self.state, "    movl %eax, {}", dsr0);
            }
        }

        // Load x_hi into %eax and clear sign bit (bit 31)
        match x {
            Operand::Const(IrConst::F64(f)) => {
                let hi = (((f.to_bits() >> 32) as u32) & 0x7FFF_FFFF) as i32;
                emit!(self.state, "    movl ${}, %eax", hi);
            }
            Operand::Value(v) if self.state.get_slot(v.0).is_some() => {
                let x_slot = self.state.get_slot(v.0).unwrap();
                let sr4 = self.slot_ref_offset(x_slot, 4);
                emit!(self.state, "    movl {}, %eax", sr4);
                self.state.emit("    andl $0x7fffffff, %eax");
            }
            _ => {
                self.state.emit("    andl $0x7fffffff, %eax");
            }
        }

        // Load y_hi sign bit into %ecx and OR into %eax
        match y {
            Operand::Const(IrConst::F64(f)) => {
                let sign = (((f.to_bits() >> 32) as u32) & 0x8000_0000) as i32;
                if sign != 0 {
                    emit!(self.state, "    orl ${}, %eax", sign);
                }
            }
            Operand::Value(v) if self.state.get_slot(v.0).is_some() => {
                let y_slot = self.state.get_slot(v.0).unwrap();
                let sr4 = self.slot_ref_offset(y_slot, 4);
                emit!(self.state, "    movl {}, %ecx", sr4);
                self.state.emit("    andl $0x80000000, %ecx");
                self.state.emit("    orl %ecx, %eax");
            }
            _ => {
                self.operand_to_ecx(y);
                self.state.emit("    andl $0x80000000, %ecx");
                self.state.emit("    orl %ecx, %eax");
            }
        }

        emit!(self.state, "    movl %eax, {}", dsr4);
        self.state.reg_cache.invalidate_all();
    }

    /// Emit copysign for 32-bit float (F32): magnitude of x, sign of y.
    fn emit_f32_copysign(&mut self, dest: &Option<Value>, x: &Operand, y: &Operand) {
        let Some(d) = dest else { return };
        let Some(dest_slot) = self.state.get_slot(d.0) else { return };
        let dsr = self.slot_ref(dest_slot);

        match x {
            Operand::Const(IrConst::F32(f)) => {
                let bits = ((f.to_bits() as u32) & 0x7FFF_FFFF) as i32;
                emit!(self.state, "    movl ${}, %eax", bits);
            }
            Operand::Value(v) if self.state.get_slot(v.0).is_some() => {
                let x_slot = self.state.get_slot(v.0).unwrap();
                let sr = self.slot_ref(x_slot);
                emit!(self.state, "    movl {}, %eax", sr);
                self.state.emit("    andl $0x7fffffff, %eax");
            }
            _ => {
                self.operand_to_eax(x);
                self.state.emit("    andl $0x7fffffff, %eax");
            }
        }

        match y {
            Operand::Const(IrConst::F32(f)) => {
                let sign = ((f.to_bits() as u32) & 0x8000_0000) as i32;
                if sign != 0 {
                    emit!(self.state, "    orl ${}, %eax", sign);
                }
            }
            Operand::Value(v) if self.state.get_slot(v.0).is_some() => {
                let y_slot = self.state.get_slot(v.0).unwrap();
                let sr = self.slot_ref(y_slot);
                emit!(self.state, "    movl {}, %ecx", sr);
                self.state.emit("    andl $0x80000000, %ecx");
                self.state.emit("    orl %ecx, %eax");
            }
            _ => {
                self.operand_to_ecx(y);
                self.state.emit("    andl $0x80000000, %ecx");
                self.state.emit("    orl %ecx, %eax");
            }
        }

        emit!(self.state, "    movl %eax, {}", dsr);
        self.state.reg_cache.invalidate_all();
    }

    /// Emit copysign for 80-bit x87 float: magnitude of x, sign of y.
    fn emit_ld_copysign(&mut self, dest: &Option<Value>, x: &Operand, y: &Operand) {
        let Some(d) = dest else { return };
        let Some(dest_slot) = self.state.get_slot(d.0) else { return };
        let dsr0 = self.slot_ref(dest_slot);
        let dsr4 = self.slot_ref_offset(dest_slot, 4);
        let dsr8 = self.slot_ref_offset(dest_slot, 8);

        if let Operand::Value(xv) = x {
            if let Some(x_slot) = self.state.get_slot(xv.0) {
                let xsr0 = self.slot_ref(x_slot);
                let xsr4 = self.slot_ref_offset(x_slot, 4);
                let xsr8 = self.slot_ref_offset(x_slot, 8);
                emit!(self.state, "    movl {}, %eax", xsr0);
                emit!(self.state, "    movl %eax, {}", dsr0);
                emit!(self.state, "    movl {}, %eax", xsr4);
                emit!(self.state, "    movl %eax, {}", dsr4);
                emit!(self.state, "    movzwl {}, %eax", xsr8);
                self.state.emit("    andl $0x7fff, %eax");
            }
        }
        if let Operand::Value(yv) = y {
            if let Some(y_slot) = self.state.get_slot(yv.0) {
                let ysr8 = self.slot_ref_offset(y_slot, 8);
                emit!(self.state, "    movzwl {}, %ecx", ysr8);
                self.state.emit("    andl $0x8000, %ecx");
                self.state.emit("    orl %ecx, %eax");
            }
        }
        emit!(self.state, "    movw %ax, {}", dsr8);
        self.state.f128_direct_slots.insert(d.0);
        self.state.reg_cache.invalidate_all();
    }

    /// IEEE binary128 helpers.  On i686 a _Float128 value is a 16-byte
    /// binary128 bit pattern carried in a U128 container (soft-float via the
    /// libgcc TF helpers), so the sign bit is bit 127 — the top bit of the
    /// dword at byte offset 12 — NOT the x87 sign word at byte offset 8.
    /// Results are marked in `i128_values` (matching the x86-64 arms) so
    /// every copy/store/load path treats them as full 128-bit carriers.
    const F128_SIGN_DWORD_OFF: i64 = 12;

    /// Extract the four little-endian dwords of a 16-byte constant payload.
    /// `IrConst::I128` is the container form produced by the _Float128
    /// lowering; `IrConst::LongDouble(_, bytes)` also carries the full
    /// 16-byte binary128 pattern (see const_arith) and `Zero` is +0.
    fn f128_const_words(op: &Operand) -> Option<[u32; 4]> {
        match op {
            Operand::Const(IrConst::I128(v)) => {
                let b = v.to_le_bytes();
                Some([
                    u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                    u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
                    u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
                    u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
                ])
            }
            Operand::Const(IrConst::LongDouble(_, bytes)) => Some([
                u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
                u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
                u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
            ]),
            Operand::Const(IrConst::Zero) => Some([0; 4]),
            _ => None,
        }
    }

    fn emit_f128_store_words(&mut self, words: &[u32; 4], dslot: StackSlot) {
        for (k, w) in words.iter().enumerate() {
            let dsr = self.slot_ref_offset(dslot, 4 * k as i64);
            emit!(self.state, "    movl ${}, {}", w, dsr);
        }
    }

    /// _Float128 negation/fabs: btc/btr of binary128 sign bit 127.
    /// Constants fold in IR-space (including -0.0: negating Zero sets the
    /// sign bit, exactly like the x86-64 btcq contract).
    fn emit_f128_signop(&mut self, dest: &Option<Value>, x: &Operand, abs: bool) {
        let Some(d) = dest else { return };
        let Some(dest_slot) = self.state.get_slot(d.0) else { return };
        let mnem = if abs { "btrl" } else { "btcl" };
        if let Some(mut words) = Self::f128_const_words(x) {
            if abs {
                words[3] &= 0x7FFF_FFFF;
            } else {
                words[3] ^= 0x8000_0000;
            }
            self.emit_f128_store_words(&words, dest_slot);
        } else if let Operand::Value(xv) = x {
            let Some(x_slot) = self.state.get_slot(xv.0) else { return };
            // Copy the three low dwords, then transform the sign dword.
            for k in 0..3i64 {
                let sr = self.slot_ref_offset(x_slot, 4 * k);
                let dsr = self.slot_ref_offset(dest_slot, 4 * k);
                emit!(self.state, "    movl {}, %eax", sr);
                emit!(self.state, "    movl %eax, {}", dsr);
            }
            let xsr = self
                .slot_ref_offset(x_slot, Self::F128_SIGN_DWORD_OFF);
            let dsr = self
                .slot_ref_offset(dest_slot, Self::F128_SIGN_DWORD_OFF);
            emit!(self.state, "    movl {}, %eax", xsr);
            emit!(self.state, "    {} $31, %eax", mnem);
            emit!(self.state, "    movl %eax, {}", dsr);
        } else {
            return;
        }
        self.state.i128_values.insert(d.0);
        self.state.reg_cache.invalidate_all();
    }

    /// _Float128 copysign: high dword = (x & ~sign) | (y & sign), low 12
    /// bytes = x. Pure GPR; `shll $31` isolates y's sign bit.
    fn emit_f128_copysign(&mut self, dest: &Option<Value>, x: &Operand, y: &Operand) {
        let Some(d) = dest else { return };
        let Some(dest_slot) = self.state.get_slot(d.0) else { return };
        let xw = Self::f128_const_words(x);
        let yw = Self::f128_const_words(y);
        if let (Some(xw_v), Some(yw_v)) = (&xw, &yw) {
            // Fully constant: fold to the exact binary128 pattern.
            let mut folded = *xw_v;
            folded[3] = (folded[3] & 0x7FFF_FFFF) | (yw_v[3] & 0x8000_0000);
            self.emit_f128_store_words(&folded, dest_slot);
        } else {
            // Low 12 bytes: copy from x (constant x stores immediates).
            match (&xw, x) {
                (Some(words), _) => self.emit_f128_store_words(&[words[0], words[1], words[2], 0], dest_slot),
                _ => {
                    if let Operand::Value(xv) = x {
                        if let Some(x_slot) = self.state.get_slot(xv.0) {
                            for k in 0..3i64 {
                                let sr = self.slot_ref_offset(x_slot, 4 * k);
                                let dsr = self.slot_ref_offset(dest_slot, 4 * k);
                                emit!(self.state, "    movl {}, %eax", sr);
                                emit!(self.state, "    movl %eax, {}", dsr);
                            }
                        }
                    }
                }
            }
            // Sign dword: magnitude from x, sign from y.
            let dsr = self
                .slot_ref_offset(dest_slot, Self::F128_SIGN_DWORD_OFF);
            match (&xw, x) {
                (Some(words), _) => {
                    emit!(self.state, "    movl ${}, %eax", words[3] & 0x7FFF_FFFF);
                }
                _ => {
                    if let Operand::Value(xv) = x {
                        if let Some(x_slot) = self.state.get_slot(xv.0) {
                            let xsr = self.slot_ref_offset(x_slot, Self::F128_SIGN_DWORD_OFF);
                            emit!(self.state, "    movl {}, %eax", xsr);
                            self.state.emit("    andl $0x7fffffff, %eax");
                        }
                    }
                }
            }
            match (&yw, y) {
                (Some(words), _) => {
                    if words[3] & 0x8000_0000 != 0 {
                        emit!(self.state, "    orl ${}, %eax", 0x8000_0000u32 as i32);
                    }
                }
                _ => {
                    if let Operand::Value(yv) = y {
                        if let Some(y_slot) = self.state.get_slot(yv.0) {
                            let ysr = self.slot_ref_offset(y_slot, Self::F128_SIGN_DWORD_OFF);
                            emit!(self.state, "    movl {}, %ecx", ysr);
                            self.state.emit("    shll $31, %ecx");
                            self.state.emit("    orl %ecx, %eax");
                        }
                    }
                }
            }
            emit!(self.state, "    movl %eax, {}", dsr);
        }
        self.state.i128_values.insert(d.0);
        self.state.reg_cache.invalidate_all();
    }

    /// long double (80-bit x87, 10 bytes in a 16-byte slot) fabs: clear bit
    /// 79 — byte 9 bit 7. Pure GPR, no x87 round-trip (x86-64 contract).
    fn emit_ld_fabs(&mut self, dest: &Option<Value>, x: &Operand) {
        let Some(d) = dest else { return };
        let Some(dest_slot) = self.state.get_slot(d.0) else { return };
        if let Operand::Const(IrConst::LongDouble(_, bytes)) = x {
            let mut b = *bytes;
            b[9] &= 0x7f;
            let words = [
                u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            ];
            emit!(self.state, "    movl ${}, {}", words[0], self.slot_ref_offset(dest_slot, 0));
            emit!(self.state, "    movl ${}, {}", words[1], self.slot_ref_offset(dest_slot, 4));
            let b8 = i32::from(b[8]);
            let b9 = i32::from(b[9]);
            emit!(self.state, "    movl ${}, {}", b8 | (b9 << 8), self.slot_ref_offset(dest_slot, 8));
        } else if let Operand::Value(xv) = x {
            let Some(x_slot) = self.state.get_slot(xv.0) else { return };
            let xsr0 = self.slot_ref_offset(x_slot, 0);
            let xsr4 = self.slot_ref_offset(x_slot, 4);
            let xsr8 = self.slot_ref_offset(x_slot, 8);
            let xsr9 = self.slot_ref_offset(x_slot, 9);
            let dsr0 = self.slot_ref_offset(dest_slot, 0);
            let dsr4 = self.slot_ref_offset(dest_slot, 4);
            let dsr8 = self.slot_ref_offset(dest_slot, 8);
            let dsr9 = self.slot_ref_offset(dest_slot, 9);
            emit!(self.state, "    movl {}, %eax", xsr0);
            emit!(self.state, "    movl %eax, {}", dsr0);
            emit!(self.state, "    movl {}, %eax", xsr4);
            emit!(self.state, "    movl %eax, {}", dsr4);
            emit!(self.state, "    movzbl {}, %eax", xsr8);
            emit!(self.state, "    movb %al, {}", dsr8);
            emit!(self.state, "    movzbl {}, %eax", xsr9);
            self.state.emit("    andl $0x7f, %eax");
            emit!(self.state, "    movb %al, {}", dsr9);
        } else {
            return;
        }
        self.state.f128_direct_slots.insert(d.0);
        self.state.reg_cache.invalidate_all();
    }
}

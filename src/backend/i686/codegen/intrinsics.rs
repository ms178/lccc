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
            IntrinsicOp::F128Copysign | IntrinsicOp::LDCopysign => {
                self.emit_f128_copysign(dest, &args[0], &args[1]);
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
            | IntrinsicOp::Punpckhwd128 => {
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
                // TODO: PINSRQ is not available on i686 - could emulate with two PINSRD
                // Currently just copies input unchanged (no-op)
                if let Some(dptr) = dest_ptr {
                    self.operand_to_eax(&args[0]);
                    self.state.emit("    movdqu (%eax), %xmm0");
                    self.operand_to_eax(&Operand::Value(*dptr));
                    self.state.emit("    movdqu %xmm0, (%eax)");
                }
            }
            IntrinsicOp::Pextrq128 => {
                // TODO: PEXTRQ is not available on i686 - could emulate with MOVQ or two PEXTRD
                // Currently only extracts low 32 bits as fallback
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
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
                // On i686, only extracts the low 32 bits
                self.operand_to_eax(&args[0]);
                self.state.emit("    movdqu (%eax), %xmm0");
                self.state.emit("    movd %xmm0, %eax");
                self.state.reg_cache.invalidate_acc();
                if let Some(d) = dest {
                    self.store_eax_to(d);
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
    fn emit_f128_copysign(&mut self, dest: &Option<Value>, x: &Operand, y: &Operand) {
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
}

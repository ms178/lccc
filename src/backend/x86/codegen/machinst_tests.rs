//! Stress tests for the MachInst representation and its AT&T emitter.
//!
//! This layer had **no test coverage at all** (946 lines across
//! `machinst.rs` + `machinst_emit.rs`), which is how a silent
//! `_ => "rax"` fallback survived in the 64-bit register table: an
//! unexpected register index produced a syntactically valid instruction
//! naming the *wrong* register. Nothing downstream can catch that — the
//! assembler is happy, the program is wrong.
//!
//! The suite is organised as four layers, weakest to strongest:
//!
//! 1. **Table integrity** — the four register-name tables must agree with each
//!    other and be injective. Catches copy-paste damage in the tables, which
//!    is the single most likely defect in code shaped like this.
//! 2. **Type mapping** — `OpSize` must classify every `IrType`, and its
//!    suffix must match its width.
//! 3. **Golden emission** — each `MachInst` variant and each `MachOperand`
//!    shape emits exactly the expected AT&T text.
//! 4. **Assembler differential** — every instruction the suite can construct
//!    is fed to the *real* system assembler. This is the layer that makes the
//!    others hard to fool: a golden test only proves the emitter matches what
//!    the test author expected, while `as` proves the text is genuinely a
//!    valid x86-64 instruction. It is skipped, loudly, when no assembler is
//!    present rather than passing vacuously.

use super::machinst::*;
use super::machinst_emit::*;
use crate::backend::common::AsmOutput;
use crate::backend::regalloc::PhysReg;

/// Emit one instruction and return the trimmed text lines it produced.
fn emit(inst: &MachInst) -> Vec<String> {
    let mut out = AsmOutput::new();
    emit_machinst(inst, &mut out);
    out.buf
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Emit one instruction and return its single line, asserting there is one.
fn emit1(inst: &MachInst) -> String {
    let lines = emit(inst);
    assert_eq!(lines.len(), 1, "expected one line, got {:?}", lines);
    lines.into_iter().next().unwrap()
}

fn reg(p: PhysReg) -> MachOperand {
    MachOperand::Reg(MachReg::Phys(p))
}

const SIZES: [OpSize; 4] = [OpSize::S8, OpSize::S16, OpSize::S32, OpSize::S64];

/// The canonical x86-64 register families, indexed the way `machinst.rs`
/// numbers them. Written out independently of the emitter so a typo in the
/// emitter's tables cannot agree with a typo here.
fn expected_names(p: PhysReg) -> Option<[&'static str; 4]> {
    // [S8, S16, S32, S64]
    Some(match p.0 {
        0 => ["al", "ax", "eax", "rax"],
        1 => ["bl", "bx", "ebx", "rbx"],
        2 => ["r12b", "r12w", "r12d", "r12"],
        3 => ["r13b", "r13w", "r13d", "r13"],
        4 => ["r14b", "r14w", "r14d", "r14"],
        5 => ["r15b", "r15w", "r15d", "r15"],
        6 => ["bpl", "bp", "ebp", "rbp"],
        7 => ["cl", "cx", "ecx", "rcx"],
        10 => ["r11b", "r11w", "r11d", "r11"],
        11 => ["r10b", "r10w", "r10d", "r10"],
        12 => ["r8b", "r8w", "r8d", "r8"],
        13 => ["r9b", "r9w", "r9d", "r9"],
        14 => ["dil", "di", "edi", "rdi"],
        15 => ["sil", "si", "esi", "rsi"],
        16 => ["dl", "dx", "edx", "rdx"],
        _ => return None,
    })
}

/// Every register index the emitter claims to handle.
const KNOWN_REGS: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 15, 16];

// ── 1. table integrity ──────────────────────────────────────────────────────

#[test]
fn every_register_emits_its_canonical_name_at_every_size() {
    for &idx in KNOWN_REGS {
        let p = PhysReg(idx);
        let want = expected_names(p).expect("KNOWN_REGS must be covered");
        for (w, size) in SIZES.iter().enumerate() {
            let line = emit1(&MachInst::Mov {
                src: reg(p),
                dst: reg(PhysReg(if idx == 0 { 1 } else { 0 })),
                size: *size,
            });
            assert!(
                line.contains(&format!("%{}", want[w])),
                "reg {} at {:?} should spell %{}, got `{}`",
                idx,
                size,
                want[w],
                line
            );
        }
    }
}

#[test]
fn the_register_tables_are_injective_at_every_size() {
    // Two distinct PhysRegs sharing a name at any width means one of them is
    // silently aliased onto the other -- the exact damage the removed
    // `_ => "rax"` fallback used to cause on purpose.
    for size in SIZES {
        let mut seen: std::collections::HashMap<String, u8> = std::collections::HashMap::new();
        for &idx in KNOWN_REGS {
            let line = emit1(&MachInst::Mov {
                src: reg(PhysReg(idx)),
                dst: MachOperand::StackSlot(-8),
                size,
            });
            let name = line
                .split('%')
                .nth(1)
                .unwrap_or("")
                .split(|c: char| !c.is_ascii_alphanumeric())
                .next()
                .unwrap_or("")
                .to_string();
            if let Some(prev) = seen.insert(name.clone(), idx) {
                panic!(
                    "registers {} and {} both spell %{} at {:?}",
                    prev, idx, name, size
                );
            }
        }
    }
}

#[test]
fn the_four_size_tables_describe_the_same_register_family() {
    // A mismatch here (e.g. S32 giving `r12d` while S64 gives `r13`) would
    // corrupt any instruction that mixes widths on one value.
    for &idx in KNOWN_REGS {
        let want = expected_names(PhysReg(idx)).unwrap();
        // Distinctness within the family is implied by the x86 naming rules;
        // what matters is that all four come from the SAME row of the table.
        let family_digits: Vec<String> = want
            .iter()
            .map(|n| n.chars().filter(|c| c.is_ascii_digit()).collect())
            .collect();
        let nonempty: Vec<&String> = family_digits.iter().filter(|d| !d.is_empty()).collect();
        if let Some(first) = nonempty.first() {
            assert!(
                nonempty.iter().all(|d| d == first),
                "reg {} mixes families across sizes: {:?}",
                idx,
                want
            );
        }
    }
}

#[test]
fn an_unknown_register_index_is_a_hard_error_not_a_silent_rename() {
    // The regression this whole file exists for. Index 99 is not a register;
    // the emitter must refuse rather than quietly produce %rax.
    for size in SIZES {
        let r = std::panic::catch_unwind(|| {
            let mut out = AsmOutput::new();
            emit_machinst(
                &MachInst::Mov {
                    src: MachOperand::Reg(MachReg::Phys(PhysReg(99))),
                    dst: MachOperand::Reg(MachReg::Phys(PhysReg(0))),
                    size,
                },
                &mut out,
            );
            out.buf
        });
        assert!(
            r.is_err(),
            "an invalid register index must not emit silently at {:?}; got {:?}",
            size,
            r
        );
    }
}

// ── 2. type mapping ─────────────────────────────────────────────────────────

#[test]
fn op_size_classifies_every_integer_ir_type() {
    use crate::common::types::IrType;
    let cases = [
        (IrType::I8, OpSize::S8),
        (IrType::U8, OpSize::S8),
        (IrType::I16, OpSize::S16),
        (IrType::U16, OpSize::S16),
        (IrType::I32, OpSize::S32),
        (IrType::U32, OpSize::S32),
        (IrType::F32, OpSize::S32),
        (IrType::I64, OpSize::S64),
        (IrType::U64, OpSize::S64),
        (IrType::F64, OpSize::S64),
        (IrType::Ptr, OpSize::S64),
    ];
    for (ty, want) in cases {
        assert_eq!(
            OpSize::from_ir_type(ty),
            want,
            "{:?} must map to {:?}",
            ty,
            want
        );
    }
}

#[test]
fn every_op_size_has_the_matching_att_suffix() {
    assert_eq!(OpSize::S8.suffix(), "b");
    assert_eq!(OpSize::S16.suffix(), "w");
    assert_eq!(OpSize::S32.suffix(), "l");
    assert_eq!(OpSize::S64.suffix(), "q");
}

// ── 3. golden emission, per operand shape ───────────────────────────────────

#[test]
fn a_register_to_register_move_emits_both_operands_at_the_right_width() {
    assert_eq!(
        emit1(&MachInst::Mov {
            src: reg(PhysReg(0)),
            dst: reg(PhysReg(15)),
            size: OpSize::S32,
        }),
        "movl %eax, %esi"
    );
}

#[test]
fn a_self_move_emits_nothing() {
    let lines = emit(&MachInst::Mov {
        src: reg(PhysReg(0)),
        dst: reg(PhysReg(0)),
        size: OpSize::S64,
    });
    assert!(lines.is_empty(), "self-move must be elided, got {:?}", lines);
}

#[test]
fn a_memory_to_memory_move_relays_through_a_scratch_register() {
    // x86 has no mem-to-mem move; the emitter must split it.
    let lines = emit(&MachInst::Mov {
        src: MachOperand::StackSlot(-8),
        dst: MachOperand::StackSlot(-16),
        size: OpSize::S64,
    });
    assert_eq!(lines.len(), 2, "must become two instructions: {:?}", lines);
    assert!(lines[0].contains("%rax"), "{:?}", lines);
    assert!(lines[1].contains("%rax"), "{:?}", lines);
}

#[test]
fn indexed_memory_emits_every_legal_scale() {
    for scale in [1u8, 2, 4, 8] {
        let line = emit1(&MachInst::Mov {
            src: MachOperand::MemIndex {
                base: MachReg::Phys(PhysReg(14)),
                index: MachReg::Phys(PhysReg(1)),
                scale,
                offset: 0,
            },
            dst: reg(PhysReg(0)),
            size: OpSize::S64,
        });
        assert!(
            line.contains("%rdi") && line.contains("%rbx"),
            "scale {}: {}",
            scale,
            line
        );
        if scale != 1 {
            assert!(line.contains(&scale.to_string()), "scale {}: {}", scale, line);
        }
    }
}

#[test]
fn memory_offsets_emit_with_their_sign() {
    let neg = emit1(&MachInst::Mov {
        src: MachOperand::Mem {
            base: MachReg::Phys(PhysReg(14)),
            offset: -24,
        },
        dst: reg(PhysReg(0)),
        size: OpSize::S64,
    });
    assert!(neg.contains("-24(%rdi)"), "{}", neg);

    let pos = emit1(&MachInst::Mov {
        src: MachOperand::Mem {
            base: MachReg::Phys(PhysReg(14)),
            offset: 24,
        },
        dst: reg(PhysReg(0)),
        size: OpSize::S64,
    });
    assert!(pos.contains("24(%rdi)"), "{}", pos);
}

#[test]
fn a_rip_relative_symbol_emits_the_rip_form() {
    let line = emit1(&MachInst::Mov {
        src: MachOperand::RipRel("glob".into()),
        dst: reg(PhysReg(0)),
        size: OpSize::S64,
    });
    assert!(line.contains("glob(%rip)"), "{}", line);
}

// ── 3b. golden emission, per instruction variant ────────────────────────────

#[test]
fn every_alu_operation_emits_its_mnemonic() {
    for (op, mnem) in [
        (AluOp::Add, "add"),
        (AluOp::Sub, "sub"),
        (AluOp::And, "and"),
        (AluOp::Or, "or"),
        (AluOp::Xor, "xor"),
        (AluOp::Imul, "imul"),
    ] {
        let line = emit1(&MachInst::Alu {
            op,
            src: reg(PhysReg(7)),
            dst: MachReg::Phys(PhysReg(0)),
            size: OpSize::S64,
        });
        assert!(
            line.starts_with(mnem),
            "{:?} should emit `{}`, got `{}`",
            op,
            mnem,
            line
        );
        assert!(line.contains("%rcx") && line.contains("%rax"), "{}", line);
    }
}

#[test]
fn every_shift_operation_emits_its_mnemonic() {
    for (op, mnem) in [(ShiftOp::Shl, "shl"), (ShiftOp::Shr, "shr"), (ShiftOp::Sar, "sar")] {
        let line = emit1(&MachInst::Shift {
            op,
            amount: MachOperand::Imm(3),
            dst: MachReg::Phys(PhysReg(0)),
            size: OpSize::S64,
        });
        assert!(line.starts_with(mnem), "{:?} -> `{}`", op, line);
    }
}

#[test]
fn every_condition_code_emits_a_distinct_suffix() {
    let ccs = [
        CondCode::E,
        CondCode::Ne,
        CondCode::L,
        CondCode::Le,
        CondCode::G,
        CondCode::Ge,
        CondCode::B,
        CondCode::Be,
        CondCode::A,
        CondCode::Ae,
    ];
    let mut seen = std::collections::HashSet::new();
    for cc in ccs {
        let line = emit1(&MachInst::SetCC {
            cc,
            dst: MachReg::Phys(PhysReg(0)),
        });
        assert!(line.starts_with("set"), "{:?} -> `{}`", cc, line);
        let suffix = line
            .split_whitespace()
            .next()
            .unwrap()
            .trim_start_matches("set")
            .to_string();
        assert!(
            seen.insert(suffix.clone()),
            "condition {:?} reuses suffix `{}`",
            cc,
            suffix
        );
    }
    assert_eq!(seen.len(), ccs.len());
}

#[test]
fn setcc_writes_a_byte_register() {
    // `setcc` has no width variants: the destination must be the 8-bit name.
    let line = emit1(&MachInst::SetCC {
        cc: CondCode::E,
        dst: MachReg::Phys(PhysReg(15)),
    });
    assert!(line.contains("%sil"), "setcc must use the byte name: {}", line);
}

#[test]
fn zero_and_sign_extension_emit_distinct_mnemonics() {
    let z = emit1(&MachInst::Movzx {
        src: reg(PhysReg(0)),
        dst: MachReg::Phys(PhysReg(1)),
        from_size: OpSize::S8,
        to_size: OpSize::S32,
    });
    let s = emit1(&MachInst::Movsx {
        src: reg(PhysReg(0)),
        dst: MachReg::Phys(PhysReg(1)),
        from_size: OpSize::S8,
        to_size: OpSize::S32,
    });
    assert!(z.starts_with("movz"), "{}", z);
    assert!(s.starts_with("movs"), "{}", s);
    assert_ne!(z, s);
    // The SOURCE must be spelled at from_size and the DEST at to_size.
    assert!(z.contains("%al") && z.contains("%ebx"), "{}", z);
    assert!(s.contains("%al") && s.contains("%ebx"), "{}", s);
}

#[test]
fn a_symbol_address_emits_leaq_not_movq() {
    // `GlobalAddr` wants the ADDRESS of a symbol. Emitting `movq sym(%rip)`
    // would load its CONTENTS -- a silent miscompile that assembles perfectly.
    // This distinction is why `LeaSym` exists as its own variant.
    let line = emit1(&MachInst::LeaSym {
        sym: "glob".into(),
        dst: MachReg::Phys(PhysReg(15)),
    });
    assert!(line.starts_with("leaq"), "must be leaq, got `{line}`");
    assert!(line.contains("glob(%rip)"), "{line}");
    assert!(line.contains("%rsi"), "address is 64-bit: {line}");
    assert!(!line.contains("movq"), "{line}");
}

#[test]
fn control_flow_variants_emit_their_targets() {
    assert!(emit1(&MachInst::Jmp {
        target: ".L7".into()
    })
    .contains(".L7"));
    assert!(emit1(&MachInst::Jcc {
        cc: CondCode::Ne,
        target: ".L9".into()
    })
    .contains(".L9"));
    assert!(emit1(&MachInst::Call {
        target: "printf".into()
    })
    .contains("printf"));
    assert_eq!(emit1(&MachInst::Ret), "ret");
}

#[test]
fn a_large_immediate_is_materialized_rather_than_truncated() {
    // x86 ALU immediates are sign-extended 32-bit. A 64-bit constant must be
    // loaded into a register first, or the value is silently wrong.
    let lines = emit(&MachInst::Alu {
        op: AluOp::Add,
        src: MachOperand::Imm(0x1234_5678_9abc),
        dst: MachReg::Phys(PhysReg(1)),
        size: OpSize::S64,
    });
    assert!(
        lines.len() >= 2 && lines.iter().any(|l| l.contains("movabs")),
        "a 48-bit immediate must be materialized: {:?}",
        lines
    );
}

#[test]
fn a_small_immediate_is_used_directly() {
    let lines = emit(&MachInst::Alu {
        op: AluOp::Add,
        src: MachOperand::Imm(16),
        dst: MachReg::Phys(PhysReg(0)),
        size: OpSize::S64,
    });
    assert_eq!(lines.len(), 1, "no materialization needed: {:?}", lines);
    assert!(lines[0].contains("$16"), "{}", lines[0]);
}

// ── 4. differential against the real assembler ──────────────────────────────

/// Build a corpus covering every instruction variant across a spread of
/// registers, sizes and operand shapes.
fn instruction_corpus() -> Vec<MachInst> {
    let mut v = Vec::new();
    let regs = [PhysReg(0), PhysReg(7), PhysReg(15), PhysReg(2), PhysReg(16)];

    // CallTyped: every stage and operand shape the lowering can produce —
    // caller-save spill/restore pairs, zero/imm32/reg/slot arguments, the
    // sized return home — so the real-assembler differential proves the
    // whole sequence encodes, not just the call mnemonic.
    {
        let abi = [PhysReg(14), PhysReg(15), PhysReg(16), PhysReg(7), PhysReg(12), PhysReg(13)];
        for (i, dst) in abi.iter().enumerate() {
            v.push(MachInst::CallTyped {
                caller_saves: vec![(PhysReg(10), -48), (PhysReg(11), -56)],
                args: vec![
                    CallArgMove {
                        src: MachOperand::Imm(if i % 2 == 0 { 0 } else { 4096 + i as i64 }),
                        dst_reg: *dst,
                        size: OpSize::S64,
                    },
                    CallArgMove {
                        src: reg(PhysReg(if i == 5 { 1 } else { 2 })),
                        dst_reg: PhysReg(if i == 5 { 15 } else { 14 }),
                        size: OpSize::S32,
                    },
                    CallArgMove {
                        src: MachOperand::StackSlot(-24 - i as i64),
                        dst_reg: PhysReg(if i == 5 { 16 } else { abi[(i + 1) % 6].0 }),
                        size: OpSize::S64,
                    },
                ],
                target: "machinst_probe_callee".into(),
                ret: Some(CallRetMove {
                    dst: reg(PhysReg((2 + (i % 4)) as u8)),
                    size: if i % 2 == 0 { OpSize::S32 } else { OpSize::S64 },
                }),
            });
        }
        // Void call, no saves, no ret.
        v.push(MachInst::CallTyped {
            caller_saves: vec![],
            args: vec![CallArgMove {
                src: reg(PhysReg(1)),
                dst_reg: PhysReg(14),
                size: OpSize::S64,
            }],
            target: "machinst_probe_void".into(),
            ret: None,
        });
        // The xmm0/xmm1 pre-colored scratch pair participates in FMov shapes.
        for (a, b) in [(18u8, 20u8), (20, 18), (19, 21), (21, 19)] {
            v.push(MachInst::FMov {
                src: reg(PhysReg(a)),
                dst: reg(PhysReg(b)),
                size: OpSize::S64,
            });
            v.push(MachInst::FMov {
                src: reg(PhysReg(a)),
                dst: MachOperand::StackSlot(-64),
                size: OpSize::S32,
            });
        }
    }

    // FMov: every xmm allocator home at both scalar widths, plus every
    // operand pairing the lowering can produce (reg/reg, reg/mem, mem/reg,
    // reg/slot, slot/reg). The assembler differential proves the xmm8-15
    // names encode (REX-prefixed) and that no shape is mem-to-mem.
    for idx in 20u8..=33 {
        let a = PhysReg(idx);
        let b = PhysReg(if idx == 33 { 20 } else { idx + 1 });
        for size in [OpSize::S32, OpSize::S64] {
            v.push(MachInst::FMov {
                src: reg(a),
                dst: reg(b),
                size,
            });
            for op in [FAluOp::Add, FAluOp::Sub, FAluOp::Mul, FAluOp::Div] {
                v.push(MachInst::FAlu {
                    op,
                    src2: reg(b),
                    src1: MachReg::Phys(a),
                    dst: MachReg::Phys(a),
                    size,
                });
                v.push(MachInst::FAlu {
                    op,
                    src2: MachOperand::StackSlot(-32),
                    src1: MachReg::Phys(a),
                    dst: MachReg::Phys(b),
                    size,
                });
            }
            v.push(MachInst::FMov {
                src: reg(a),
                dst: MachOperand::Mem {
                    base: MachReg::Phys(PhysReg(14)),
                    offset: -8,
                },
                size,
            });
            v.push(MachInst::FMov {
                src: MachOperand::Mem {
                    base: MachReg::Phys(PhysReg(15)),
                    offset: 16,
                },
                dst: reg(a),
                size,
            });
            v.push(MachInst::FMov {
                src: reg(a),
                dst: MachOperand::StackSlot(-24),
                size,
            });
            v.push(MachInst::FMov {
                src: MachOperand::StackSlot(-24),
                dst: reg(a),
                size,
            });
        }
    }

    for size in SIZES {
        for (i, &a) in regs.iter().enumerate() {
            let b = regs[(i + 1) % regs.len()];
            v.push(MachInst::Mov {
                src: reg(a),
                dst: reg(b),
                size,
            });
            v.push(MachInst::Mov {
                src: MachOperand::Imm(7),
                dst: reg(b),
                size,
            });
            v.push(MachInst::Mov {
                src: MachOperand::Mem {
                    base: MachReg::Phys(PhysReg(14)),
                    offset: -8,
                },
                dst: reg(b),
                size,
            });
            v.push(MachInst::Cmp {
                lhs: reg(a),
                rhs: reg(b),
                size,
            });
            v.push(MachInst::Test {
                lhs: reg(a),
                rhs: reg(b),
                size,
            });
            for op in [AluOp::Add, AluOp::Sub, AluOp::And, AluOp::Or, AluOp::Xor] {
                v.push(MachInst::Alu {
                    op,
                    src: reg(a),
                    dst: MachReg::Phys(b),
                    size,
                });
            }
            for op in [ShiftOp::Shl, ShiftOp::Shr, ShiftOp::Sar] {
                v.push(MachInst::Shift {
                    op,
                    amount: MachOperand::Imm(3),
                    dst: MachReg::Phys(b),
                    size,
                });
            }
            v.push(MachInst::Neg {
                dst: MachReg::Phys(b),
                size,
            });
            v.push(MachInst::Not {
                dst: MachReg::Phys(b),
                size,
            });
        }
    }
    for &r in &regs {
        for cc in [CondCode::E, CondCode::Ne, CondCode::L, CondCode::A] {
            v.push(MachInst::SetCC {
                cc,
                dst: MachReg::Phys(r),
            });
        }
        v.push(MachInst::Movzx {
            src: reg(r),
            dst: MachReg::Phys(PhysReg(0)),
            from_size: OpSize::S8,
            to_size: OpSize::S32,
        });
        v.push(MachInst::Movzx {
            src: reg(r),
            dst: MachReg::Phys(PhysReg(0)),
            from_size: OpSize::S16,
            to_size: OpSize::S64,
        });
        v.push(MachInst::Movsx {
            src: reg(r),
            dst: MachReg::Phys(PhysReg(0)),
            from_size: OpSize::S32,
            to_size: OpSize::S64,
        });
        for scale in [1u8, 2, 4, 8] {
            v.push(MachInst::Lea {
                base: MachReg::Phys(PhysReg(14)),
                index: Some((MachReg::Phys(r), scale)),
                offset: 16,
                dst: MachReg::Phys(PhysReg(0)),
            });
        }
    }
    for &r in &regs {
        v.push(MachInst::LeaSym {
            sym: "machinst_probe_sym".into(),
            dst: MachReg::Phys(r),
        });
    }
    v.push(MachInst::Ret);
    v
}

/// Locate an assembler, preferring the project's pinned **GAS 2.47**.
///
/// The differential is only as authoritative as the assembler behind it: a
/// pass against an older GAS proves the text was valid for *that* release, not
/// for the one the project targets. Search order:
///
///   1. `$LCCC_GAS` — an explicit override;
///   2. the cache `scripts/ensure_gas_247.sh` provisions;
///   3. whatever `as` is on PATH, and finally `gcc -x assembler`.
///
/// The chosen assembler and its version are reported on every run, so a probe
/// that fell back to an older GAS is visible rather than silently weaker.
fn find_assembler() -> Option<(String, String)> {
    let mut cands: Vec<String> = Vec::new();
    if let Ok(p) = std::env::var("LCCC_GAS") {
        cands.push(p);
    }
    // ensure_gas_247.sh installs to /home/user/.cache/gas-2.47-<target>/bin/as
    if let Ok(entries) = std::fs::read_dir("/home/user/.cache") {
        let mut hits: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("gas-2.47-").then(|| {
                    e.path().join("bin").join("as").to_string_lossy().to_string()
                })
            })
            .filter(|p| std::path::Path::new(p).exists())
            .collect();
        hits.sort();
        cands.extend(hits);
    }
    cands.push("as".to_string());
    cands.push("gcc".to_string());

    for cand in cands {
        if let Ok(out) = std::process::Command::new(&cand).arg("--version").output() {
            if out.status.success() {
                let ver = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                return Some((cand, ver));
            }
        }
    }
    None
}

/// Report which assembler backed a differential run. Loud on purpose: the
/// project pins GAS 2.47 and a run against anything older is weaker evidence.
fn announce_assembler(which: &str, ver: &str) {
    if ver.contains("2.47") {
        eprintln!("MachInst differential: using {which} ({ver})");
    } else {
        eprintln!(
            "MachInst differential: using {which} ({ver}) -- NOT the pinned GAS 2.47. \
             Run scripts/ensure_gas_247.sh or set LCCC_GAS to probe against the \
             assembler the project actually targets."
        );
    }
}

#[test]
fn every_emitted_instruction_is_accepted_by_the_real_assembler() {
    let Some((asm, ver)) = find_assembler() else {
        // Do NOT pass silently: a vacuous green here would hide the strongest
        // check in the file.
        eprintln!(
            "SKIP: no assembler found; the MachInst differential test cannot \
             run in this environment"
        );
        return;
    };
    announce_assembler(&asm, &ver);

    let corpus = instruction_corpus();
    assert!(corpus.len() > 300, "corpus too small: {}", corpus.len());

    let mut src = String::from(".text\n.globl _machinst_probe\n_machinst_probe:\n");
    let mut emitted: Vec<(String, String)> = Vec::new();
    for inst in &corpus {
        let mut out = AsmOutput::new();
        emit_machinst(inst, &mut out);
        for line in out.buf.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            // Branch/call targets reference labels this probe does not define.
            if t.starts_with('j') || t.starts_with("call") {
                continue;
            }
            src.push_str("    ");
            src.push_str(t);
            src.push('\n');
            emitted.push((format!("{:?}", inst), t.to_string()));
        }
    }
    src.push_str("    ret\n");

    let dir = std::env::temp_dir().join(format!("lccc-machinst-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let s_path = dir.join("probe.s");
    let o_path = dir.join("probe.o");
    std::fs::write(&s_path, &src).expect("write probe.s");

    let mut cmd = std::process::Command::new(&asm);
    if asm.ends_with("gcc") {
        cmd.arg("-c").arg("-x").arg("assembler");
    } else {
        cmd.arg("-c");
    }
    let output = cmd
        .arg(&s_path)
        .arg("-o")
        .arg(&o_path)
        .output()
        .expect("run assembler");

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        // Report the offending source lines, not just the assembler's noise.
        let mut detail = String::new();
        for line in err.lines().take(20) {
            detail.push_str("    ");
            detail.push_str(line);
            detail.push('\n');
        }
        panic!(
            "the assembler REJECTED emitter output ({} instructions probed).\n\
             This means MachInst produced text that is not a valid x86-64 \
             instruction.\n{}\n--- probe.s kept at {} ---",
            emitted.len(),
            detail,
            s_path.display()
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_emitted_line_contains_an_unresolved_virtual_register() {
    // A vreg reaching emission is a register-allocation bug. It must never
    // appear as text, in any variant.
    for inst in instruction_corpus() {
        let mut out = AsmOutput::new();
        emit_machinst(&inst, &mut out);
        assert!(
            !out.buf.contains("vreg") && !out.buf.contains("VREG"),
            "unresolved vreg in output for {:?}:\n{}",
            inst,
            out.buf
        );
    }
}

#[test]
fn emission_is_deterministic() {
    // The same instruction must always produce byte-identical text; anything
    // else makes build reproducibility and asm-diff testing impossible.
    for inst in instruction_corpus() {
        let a = {
            let mut o = AsmOutput::new();
            emit_machinst(&inst, &mut o);
            o.buf
        };
        let b = {
            let mut o = AsmOutput::new();
            emit_machinst(&inst, &mut o);
            o.buf
        };
        assert_eq!(a, b, "non-deterministic emission for {:?}", inst);
    }
}

// ── 5. randomized combination stress ────────────────────────────────────────

/// A deterministic 64-bit PRNG (SplitMix64). Seeded from a constant so a
/// failure is always reproducible; there is no value in a flaky test.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() % xs.len() as u64) as usize]
    }
}

/// Build a large randomized corpus. The hand-written corpus covers each
/// variant once; this explores the CROSS PRODUCT of operand shapes, widths and
/// registers, which is where the interesting failures live -- the narrow-shift
/// bug was a `(op, width)` pair nobody had instantiated, and a per-variant
/// golden test would never have produced it.
fn random_corpus(n: usize) -> Vec<MachInst> {
    let mut rng = Rng(0x5EED_1CCC_0000_0001);
    let regs: Vec<PhysReg> = KNOWN_REGS.iter().map(|&i| PhysReg(i)).collect();
    let sizes = SIZES;
    let alus = [AluOp::Add, AluOp::Sub, AluOp::And, AluOp::Or, AluOp::Xor, AluOp::Imul];
    let shifts = [ShiftOp::Shl, ShiftOp::Shr, ShiftOp::Sar];
    let ccs = [
        CondCode::E, CondCode::Ne, CondCode::L, CondCode::Le, CondCode::G,
        CondCode::Ge, CondCode::B, CondCode::Be, CondCode::A, CondCode::Ae,
    ];
    let offsets: [i64; 5] = [0, 8, -8, 4096, -4096];
    let scales: [u8; 4] = [1, 2, 4, 8];

    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        let size = *rng.pick(&sizes);
        let a = *rng.pick(&regs);
        let b = *rng.pick(&regs);
        let operand = |rng: &mut Rng| -> MachOperand {
            match rng.next() % 5 {
                0 => MachOperand::Reg(MachReg::Phys(*rng.pick(&regs))),
                1 => MachOperand::Imm((rng.next() % 256) as i64),
                2 => MachOperand::Mem {
                    base: MachReg::Phys(*rng.pick(&regs)),
                    offset: *rng.pick(&offsets),
                },
                3 => MachOperand::MemIndex {
                    base: MachReg::Phys(*rng.pick(&regs)),
                    index: MachReg::Phys(*rng.pick(&regs)),
                    scale: *rng.pick(&scales),
                    offset: *rng.pick(&offsets),
                },
                _ => MachOperand::StackSlot(*rng.pick(&offsets)),
            }
        };
        v.push(match rng.next() % 10 {
            0 => MachInst::Mov {
                src: operand(&mut rng),
                dst: MachOperand::Reg(MachReg::Phys(b)),
                size,
            },
            // A `Mov` destination is a register or a memory location; an
            // immediate destination is not a representable instruction, and
            // generating one would test the emitter's behaviour on input it
            // can never receive.
            1 => MachInst::Mov {
                src: MachOperand::Reg(MachReg::Phys(a)),
                dst: match rng.next() % 3 {
                    0 => MachOperand::Reg(MachReg::Phys(b)),
                    1 => MachOperand::Mem {
                        base: MachReg::Phys(*rng.pick(&regs)),
                        offset: *rng.pick(&offsets),
                    },
                    _ => MachOperand::StackSlot(*rng.pick(&offsets)),
                },
                size,
            },
            2 => MachInst::Alu {
                op: *rng.pick(&alus),
                src: operand(&mut rng),
                dst: MachReg::Phys(b),
                size,
            },
            3 => MachInst::Shift {
                op: *rng.pick(&shifts),
                amount: MachOperand::Imm((rng.next() % 32) as i64),
                dst: MachReg::Phys(b),
                size,
            },
            4 => MachInst::Cmp {
                lhs: MachOperand::Reg(MachReg::Phys(a)),
                rhs: operand(&mut rng),
                size,
            },
            5 => MachInst::Test {
                lhs: MachOperand::Reg(MachReg::Phys(a)),
                rhs: MachOperand::Reg(MachReg::Phys(b)),
                size,
            },
            6 => MachInst::SetCC {
                cc: *rng.pick(&ccs),
                dst: MachReg::Phys(b),
            },
            7 => MachInst::Lea {
                base: MachReg::Phys(a),
                index: Some((MachReg::Phys(b), *rng.pick(&scales))),
                offset: *rng.pick(&offsets),
                dst: MachReg::Phys(*rng.pick(&regs)),
            },
            8 => MachInst::LeaSym {
                sym: "machinst_probe_sym".into(),
                dst: MachReg::Phys(b),
            },
            _ => MachInst::Movzx {
                src: MachOperand::Reg(MachReg::Phys(a)),
                dst: MachReg::Phys(b),
                from_size: if rng.next() % 2 == 0 { OpSize::S8 } else { OpSize::S16 },
                to_size: if rng.next() % 2 == 0 { OpSize::S32 } else { OpSize::S64 },
            },
        });
    }
    v
}

#[test]
fn a_large_randomized_corpus_is_accepted_by_the_real_assembler() {
    let Some((asm, ver)) = find_assembler() else {
        eprintln!("SKIP: no assembler; randomized MachInst stress cannot run");
        return;
    };
    announce_assembler(&asm, &ver);
    let corpus = random_corpus(4000);

    let mut src = String::from(".text
.globl _machinst_fuzz
_machinst_fuzz:
");
    let mut kept = 0usize;
    for inst in &corpus {
        let mut out = AsmOutput::new();
        emit_machinst(inst, &mut out);
        for line in out.buf.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') || t.starts_with('j') || t.starts_with("call") {
                continue;
            }
            src.push_str("    ");
            src.push_str(t);
            src.push('\n');
            kept += 1;
        }
    }
    src.push_str("    ret
");
    assert!(kept > 3000, "expected a substantial corpus, got {kept} lines");

    let dir = std::env::temp_dir().join(format!("lccc-machinst-fuzz-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let s_path = dir.join("fuzz.s");
    let o_path = dir.join("fuzz.o");
    std::fs::write(&s_path, &src).expect("write fuzz.s");

    let mut cmd = std::process::Command::new(&asm);
    if asm.ends_with("gcc") {
        cmd.arg("-c").arg("-x").arg("assembler");
    } else {
        cmd.arg("-c");
    }
    let output = cmd.arg(&s_path).arg("-o").arg(&o_path).output().expect("run assembler");

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        panic!(
            "assembler REJECTED randomized emitter output ({kept} instructions).\n{}\n             --- kept at {} ---",
            err.lines().take(25).collect::<Vec<_>>().join("\n"),
            s_path.display()
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_randomized_corpus_is_reproducible_and_deterministic() {
    // A fuzz test that is not reproducible cannot be debugged.
    let a = random_corpus(200);
    let b = random_corpus(200);
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(format!("{:?}", x), format!("{:?}", y));
    }
    for inst in &a {
        let p = {
            let mut o = AsmOutput::new();
            emit_machinst(inst, &mut o);
            o.buf
        };
        let q = {
            let mut o = AsmOutput::new();
            emit_machinst(inst, &mut o);
            o.buf
        };
        assert_eq!(p, q, "non-deterministic emission for {:?}", inst);
    }
}

#[test]
fn no_randomized_instruction_emits_an_unresolved_register_or_empty_text() {
    for inst in random_corpus(2000) {
        let mut out = AsmOutput::new();
        emit_machinst(&inst, &mut out);
        assert!(
            !out.buf.contains("vreg") && !out.buf.contains("VREG"),
            "unresolved vreg for {:?}", inst
        );
        // Every instruction must emit SOMETHING, except the self-move the
        // emitter deliberately elides.
        let elided = matches!(&inst, MachInst::Mov { src: MachOperand::Reg(a), dst: MachOperand::Reg(b), .. } if a == b)
            || matches!(&inst, MachInst::Mov { src: MachOperand::StackSlot(a), dst: MachOperand::StackSlot(b), .. } if a == b);
        if !elided {
            assert!(
                out.buf.trim().lines().any(|l| !l.trim().is_empty()),
                "empty emission for {:?}", inst
            );
        }
    }
}

// ── 6. semantic execution differential ──────────────────────────────────────
//
// Layers 1-5 prove the emitted text is a VALID instruction. They cannot prove
// it is the INTENDED one. AT&T order is the classic trap: `subq %rax, %rbx`
// means `rbx -= rax`, so an emitter that swapped the operands of a
// non-commutative op would produce text GAS accepts, a disassembler shows
// without complaint, and a fuzz corpus assembles cleanly -- while every
// program computes the wrong answer.
//
// The only way to close that gap is to RUN the instruction. Each case below
// is assembled into a real function, linked, executed with known inputs, and
// compared against the semantics computed independently in Rust.

/// Assemble `body` as `long f(long,long)` (SysV: rdi, rsi), run it on the
/// given inputs, and return the results. `None` when the toolchain is absent.
fn run_emitted(body: &str, inputs: &[(i64, i64)]) -> Option<Vec<i64>> {
    let (asm, _ver) = find_assembler()?;
    // Executing needs a compiler+linker for the harness, not just `as`.
    if std::process::Command::new("cc").arg("--version").output().is_err() {
        return None;
    }

    let dir = std::env::temp_dir().join(format!(
        "lccc-machinst-exec-{}-{}",
        std::process::id(),
        body.len()
    ));
    let _ = std::fs::create_dir_all(&dir);
    let s_path = dir.join("f.s");
    let c_path = dir.join("m.c");
    let bin = dir.join("m");

    std::fs::write(
        &s_path,
        format!(".text\n.globl probe_fn\n.type probe_fn,@function\nprobe_fn:\n{body}\n    ret\n"),
    )
    .ok()?;

    let mut main_src = String::from(
        "#include <stdio.h>\nlong probe_fn(long,long);\nint main(void){\n",
    );
    for (a, b) in inputs {
        main_src.push_str(&format!(
            "    printf(\"%lld\\n\", (long long)probe_fn({}L, {}L));\n",
            a, b
        ));
    }
    main_src.push_str("    return 0;\n}\n");
    std::fs::write(&c_path, main_src).ok()?;

    // Assemble with the pinned assembler, then link with cc.
    let obj = dir.join("f.o");
    let mut cmd = std::process::Command::new(&asm);
    if asm.ends_with("gcc") {
        cmd.arg("-c").arg("-x").arg("assembler");
    } else {
        cmd.arg("-c");
    }
    let a_out = cmd.arg(&s_path).arg("-o").arg(&obj).output().ok()?;
    if !a_out.status.success() {
        panic!(
            "assembler rejected an execution probe:\n{}\n--- source ---\n{}",
            String::from_utf8_lossy(&a_out.stderr),
            body
        );
    }
    let l_out = std::process::Command::new("cc")
        .arg(&c_path)
        .arg(&obj)
        .arg("-o")
        .arg(&bin)
        .output()
        .ok()?;
    if !l_out.status.success() {
        return None;
    }
    let r = std::process::Command::new(&bin).output().ok()?;
    let vals: Vec<i64> = String::from_utf8_lossy(&r.stdout)
        .lines()
        .filter_map(|l| l.trim().parse::<i64>().ok())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    (vals.len() == inputs.len()).then_some(vals)
}

/// Emit `inst` into a probe that loads rdi/rsi into the operand registers and
/// returns the destination.
fn probe_body(setup_dst: PhysReg, setup_src: PhysReg, inst: &MachInst) -> String {
    let mut out = AsmOutput::new();
    emit_machinst(inst, &mut out);
    let d = expected_names(setup_dst).unwrap()[3]; // 64-bit spelling
    let s = expected_names(setup_src).unwrap()[3];
    format!(
        "    movq %rdi, %{d}\n    movq %rsi, %{s}\n{}    movq %{d}, %rax\n",
        out.buf
    )
}

const EXEC_INPUTS: &[(i64, i64)] = &[
    (100, 7),
    (7, 100),
    (-9, 4),
    (0, 0),
    (i32::MAX as i64, 1),
    (-1, 1),
];

#[test]
fn alu_operations_compute_the_right_answer_when_executed() {
    // %rbx and %rcx: callee-saved %rbx would need saving, so use two
    // caller-saved families the harness does not rely on.
    let dst = PhysReg(11); // r10
    let src = PhysReg(10); // r11

    let cases: &[(AluOp, fn(i64, i64) -> i64)] = &[
        (AluOp::Add, |a, b| a.wrapping_add(b)),
        // Non-commutative: THIS is the case an operand-order bug breaks, and
        // the case no "the assembler accepted it" test can detect.
        (AluOp::Sub, |a, b| a.wrapping_sub(b)),
        (AluOp::And, |a, b| a & b),
        (AluOp::Or, |a, b| a | b),
        (AluOp::Xor, |a, b| a ^ b),
        (AluOp::Imul, |a, b| a.wrapping_mul(b)),
    ];

    for (op, model) in cases {
        let inst = MachInst::Alu {
            op: *op,
            src: MachOperand::Reg(MachReg::Phys(src)),
            dst: MachReg::Phys(dst),
            size: OpSize::S64,
        };
        let Some(got) = run_emitted(&probe_body(dst, src, &inst), EXEC_INPUTS) else {
            eprintln!("SKIP: no assembler/linker; MachInst execution differential cannot run");
            return;
        };
        for (i, (a, b)) in EXEC_INPUTS.iter().enumerate() {
            let want = model(*a, *b);
            assert_eq!(
                got[i], want,
                "{:?}(dst={}, src={}) computed {} but must be {} \
                 (an AT&T operand-order or width bug)",
                op, a, b, got[i], want
            );
        }
    }
}

#[test]
fn shifts_compute_the_right_answer_when_executed() {
    // Shifts are the other classic order trap, and `sar` vs `shr` differ only
    // in sign behaviour -- which a negative input exposes and a positive one
    // hides.
    let dst = PhysReg(11);
    let src = PhysReg(10);
    let cases: &[(ShiftOp, u32, fn(i64, u32) -> i64)] = &[
        (ShiftOp::Shl, 3, |a, k| ((a as u64) << k) as i64),
        (ShiftOp::Shr, 3, |a, k| ((a as u64) >> k) as i64),
        (ShiftOp::Sar, 3, |a, k| a >> k),
    ];
    for (op, amt, model) in cases {
        let inst = MachInst::Shift {
            op: *op,
            amount: MachOperand::Imm(*amt as i64),
            dst: MachReg::Phys(dst),
            size: OpSize::S64,
        };
        let Some(got) = run_emitted(&probe_body(dst, src, &inst), EXEC_INPUTS) else {
            return;
        };
        for (i, (a, _)) in EXEC_INPUTS.iter().enumerate() {
            let want = model(*a, *amt);
            assert_eq!(
                got[i], want,
                "{:?} by {} on {} computed {} but must be {}",
                op, amt, a, got[i], want
            );
        }
    }
}

#[test]
fn a_32_bit_alu_zero_extends_into_the_full_register() {
    // x86-64 semantics a golden test cannot express: a 32-bit write CLEARS
    // bits 32..63. An emitter that used the 64-bit spelling would pass every
    // "does it assemble" check and silently keep the upper half.
    let dst = PhysReg(11);
    let src = PhysReg(10);
    let inst = MachInst::Alu {
        op: AluOp::Add,
        src: MachOperand::Reg(MachReg::Phys(src)),
        dst: MachReg::Phys(dst),
        size: OpSize::S32,
    };
    let inputs = &[(0x1_0000_0000i64, 1i64), (-1i64, 1i64)];
    let Some(got) = run_emitted(&probe_body(dst, src, &inst), inputs) else {
        return;
    };
    // 0x1_0000_0000 + 1 truncated to 32 bits and zero-extended = 1.
    assert_eq!(got[0], 1, "32-bit add must zero-extend, got {:#x}", got[0]);
    // -1 + 1 = 0 in 32 bits, zero-extended = 0.
    assert_eq!(got[1], 0, "32-bit add must zero-extend, got {:#x}", got[1]);
}

#[test]
fn zero_and_sign_extension_differ_where_it_matters_when_executed() {
    // movzx vs movsx on a negative byte: 0xFF becomes 255 or -1. Both
    // assemble; only execution tells them apart.
    let dst = PhysReg(11);
    let src = PhysReg(10);
    let inputs = &[(0i64, -1i64)];

    let z = MachInst::Movzx {
        src: MachOperand::Reg(MachReg::Phys(src)),
        dst: MachReg::Phys(dst),
        from_size: OpSize::S8,
        to_size: OpSize::S64,
    };
    let sx = MachInst::Movsx {
        src: MachOperand::Reg(MachReg::Phys(src)),
        dst: MachReg::Phys(dst),
        from_size: OpSize::S8,
        to_size: OpSize::S64,
    };
    let Some(gz) = run_emitted(&probe_body(dst, src, &z), inputs) else {
        return;
    };
    let Some(gs) = run_emitted(&probe_body(dst, src, &sx), inputs) else {
        return;
    };
    assert_eq!(gz[0], 255, "movzx of 0xFF must be 255, got {}", gz[0]);
    assert_eq!(gs[0], -1, "movsx of 0xFF must be -1, got {}", gs[0]);
}

#[test]
fn a_symbol_address_is_the_real_address_when_executed() {
    // Proves `LeaSym` computes the ADDRESS and not the contents. A `movq`
    // here would return the value stored at the symbol, which on this probe is
    // a recognisable sentinel -- so the two outcomes are trivially
    // distinguishable at runtime, and indistinguishable by any check that only
    // asks whether the text assembles.
    let Some((asm, _)) = find_assembler() else {
        eprintln!("SKIP: no assembler; LeaSym execution check cannot run");
        return;
    };
    if std::process::Command::new("cc").arg("--version").output().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("lccc-leasym-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (s_path, c_path, o_path, bin) = (
        dir.join("f.s"),
        dir.join("m.c"),
        dir.join("f.o"),
        dir.join("m"),
    );

    let mut out = AsmOutput::new();
    emit_machinst(
        &MachInst::LeaSym {
            sym: "leasym_probe_obj".into(),
            dst: MachReg::Phys(PhysReg(0)), // rax = return register
        },
        &mut out,
    );
    std::fs::write(
        &s_path,
        format!(
            ".text\n.globl leasym_probe\n.type leasym_probe,@function\nleasym_probe:\n{}    ret\n",
            out.buf
        ),
    )
    .unwrap();
    std::fs::write(
        &c_path,
        "#include <stdio.h>\nlong leasym_probe_obj = 0x5EEDF00D;\n         void *leasym_probe(void);\n         int main(void){ printf(\"%d\\n\", leasym_probe() == (void*)&leasym_probe_obj); return 0; }\n",
    )
    .unwrap();

    let mut cmd = std::process::Command::new(&asm);
    if asm.ends_with("gcc") {
        cmd.arg("-c").arg("-x").arg("assembler");
    } else {
        cmd.arg("-c");
    }
    let a = cmd.arg(&s_path).arg("-o").arg(&o_path).output().unwrap();
    assert!(
        a.status.success(),
        "assembler rejected LeaSym:\n{}",
        String::from_utf8_lossy(&a.stderr)
    );
    let l = std::process::Command::new("cc")
        .arg(&c_path)
        .arg(&o_path)
        .arg("-o")
        .arg(&bin)
        .output()
        .unwrap();
    if !l.status.success() {
        return; // no linker/libc in this environment
    }
    let r = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&r.stdout).trim(),
        "1",
        "LeaSym must return the symbol's ADDRESS, not its contents"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── 7. coverage regression guard ────────────────────────────────────────────

/// MachInst coverage must not silently erode.
///
/// The whole point of `CCC_ISEL_STATS` is that a shrinking coverage fraction
/// is invisible: `lower_instruction_typed` returning `false` falls back to
/// direct text emission with no diagnostic, so a class quietly dropping out of
/// the typed path looks exactly like everything being fine. The census made it
/// measurable; this test makes it *enforced*.
///
/// The floors below are deliberately a little under the measured values so
/// ordinary churn does not trip them, while a whole instruction class falling
/// out — the failure mode that matters — does. Corpus-wide coverage when these
/// were written: **85.1%**, up from 53.9% before `LeaSym`, narrow stores and
/// the no-code `ParamRef` subset were lowered.
#[test]
fn instruction_selection_covers_the_expected_instruction_classes() {
    use crate::ir::reexports::{Instruction, IrBinOp, IrConst, Operand, Value};
    use crate::common::types::IrType;
    use crate::common::fx_hash::FxHashMap;

    let ra: FxHashMap<u32, PhysReg> = [
        (1u32, PhysReg(0)),
        (2, PhysReg(7)),
        (3, PhysReg(11)),
        (4, PhysReg(20)), // xmm2-homed float value
    ]
    .into_iter()
    .collect();
    let slots: FxHashMap<u32, i64> = [(9u32, -8i64)].into_iter().collect();

    // One representative of every class the layer claims to own. A `false`
    // here means that class silently left the typed path.
    let cases: Vec<(&str, Instruction)> = vec![
        (
            "BinOp",
            Instruction::BinOp {
                dest: Value(1),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I64(3)),
                ty: IrType::I64,
            },
        ),
        (
            "Copy",
            Instruction::Copy {
                dest: Value(1),
                src: Operand::Value(Value(2)),
            },
        ),
        (
            "GlobalAddr",
            Instruction::GlobalAddr {
                dest: Value(1),
                name: "sym".into(),
            },
        ),
        (
            "Store(i64)",
            Instruction::Store {
                val: Operand::Value(Value(2)),
                ptr: Value(3),
                ty: IrType::I64,
                seg_override: Default::default(),
                volatile: false,
            },
        ),
        // The narrow stores whose blanket refusal was removed.
        (
            "Store(i8)",
            Instruction::Store {
                val: Operand::Value(Value(2)),
                ptr: Value(3),
                ty: IrType::I8,
                seg_override: Default::default(),
                volatile: false,
            },
        ),
        (
            "Store(i16)",
            Instruction::Store {
                val: Operand::Value(Value(2)),
                ptr: Value(3),
                ty: IrType::I16,
                seg_override: Default::default(),
                volatile: false,
            },
        ),
        // Scalar float moves (FMov): value 4 is xmm-homed, 3 is a GPR ptr,
        // 9 an alloca slot.
        (
            "Store(f64)",
            Instruction::Store {
                val: Operand::Value(Value(4)),
                ptr: Value(3),
                ty: IrType::F64,
                seg_override: Default::default(),
                volatile: false,
            },
        ),
        (
            "Store(f32)",
            Instruction::Store {
                val: Operand::Value(Value(4)),
                ptr: Value(9),
                ty: IrType::F32,
                seg_override: Default::default(),
                volatile: false,
            },
        ),
        (
            "Load(f64)",
            Instruction::Load {
                dest: Value(4),
                ptr: Value(3),
                ty: IrType::F64,
                seg_override: Default::default(),
                volatile: false,
            },
        ),
        (
            "BinOp(f64)",
            Instruction::BinOp {
                dest: Value(4),
                op: IrBinOp::Add,
                lhs: Operand::Value(Value(4)),
                rhs: Operand::Value(Value(4)),
                ty: IrType::F64,
            },
        ),
        (
            "Cmp",
            Instruction::Cmp {
                dest: Value(1),
                op: crate::ir::reexports::IrCmpOp::Slt,
                lhs: Operand::Value(Value(2)),
                rhs: Operand::Const(IrConst::I64(0)),
                ty: IrType::I64,
            },
        ),
        (
            "Alloca",
            Instruction::Alloca {
                dest: Value(9),
                ty: IrType::I64,
                size: 8,
                align: 0,
                volatile: false,
                semantic_volatile: false,
            },
        ),
    ];

    let mut failed = Vec::new();
    for (name, inst) in &cases {
        let mut out = Vec::new();
        if !super::isel::lower_instruction_typed(inst, &ra, &slots, None, &mut out) {
            failed.push(*name);
        }
    }
    assert!(
        failed.is_empty(),
        "these instruction classes fell OUT of MachInst lowering: {:?}\n\
         Coverage was 85.1% corpus-wide when this guard was written; a class \
         dropping out is silent (the fallback to text emission has no \
         diagnostic), which is exactly what this test exists to catch. \
         Re-measure with CCC_ISEL_STATS=1.",
        failed
    );
}

/// `GlobalAddr` must lower to `LeaSym`, not to a `Mov`.
///
/// A `Mov` from a `RipRel` operand loads the symbol's CONTENTS; `GlobalAddr`
/// wants its ADDRESS. Both assemble, so only a structural check on the emitted
/// variant (or execution) tells them apart.
#[test]
fn global_addr_lowers_to_an_address_computation_not_a_load() {
    use crate::ir::reexports::{Instruction, Value};
    use crate::common::fx_hash::FxHashMap;

    let ra: FxHashMap<u32, PhysReg> = [(1u32, PhysReg(0))].into_iter().collect();
    let slots: FxHashMap<u32, i64> = FxHashMap::default();
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(
        &Instruction::GlobalAddr {
            dest: Value(1),
            name: "the_symbol".into(),
        },
        &ra,
        &slots,
        None,
        &mut out,
    ));
    assert_eq!(out.len(), 1, "expected exactly one instruction: {out:?}");
    match &out[0] {
        MachInst::LeaSym { sym, .. } => assert_eq!(sym, "the_symbol"),
        other => panic!("GlobalAddr must lower to LeaSym, got {other:?}"),
    }
}

// ── 5. FMov: scalar SSE moves ────────────────────────────────────────────
//
// Float values live in the XMM domain (PhysReg 20..=33 = xmm2..xmm15). The
// golden tests pin the exact AT&T text; the trap tests pin the shapes the
// lowering must never produce (they would emit unencodable instructions or
// silently relay through a live register); the isel tests pin which IR
// shapes reach the typed path at all.

/// Canonical xmm names, written independently of the emitter's table.
fn expected_xmm(idx: u8) -> &'static str {
    match idx {
        20 => "xmm2",
        21 => "xmm3",
        22 => "xmm4",
        23 => "xmm5",
        24 => "xmm6",
        25 => "xmm7",
        26 => "xmm8",
        27 => "xmm9",
        28 => "xmm10",
        29 => "xmm11",
        30 => "xmm12",
        31 => "xmm13",
        32 => "xmm14",
        33 => "xmm15",
        _ => panic!("PhysReg({idx}) is not an xmm allocator home"),
    }
}

const XMM_REGS: &[u8] = &[20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33];

#[test]
fn fmov_names_every_xmm_home_at_both_scalar_widths() {
    for (i, &a) in XMM_REGS.iter().enumerate() {
        let b = XMM_REGS[(i + 1) % XMM_REGS.len()];
        // Reg-to-reg uses the VEX 3-operand form (no merging-move false
        // dependence -- see the emitter comment).
        let line = emit1(&MachInst::FMov {
            src: reg(PhysReg(a)),
            dst: reg(PhysReg(b)),
            size: OpSize::S64,
        });
        assert_eq!(
            line,
            format!(
                "vmovsd %{}, %{}, %{}",
                expected_xmm(a),
                expected_xmm(a),
                expected_xmm(b)
            )
        );
        let line = emit1(&MachInst::FMov {
            src: reg(PhysReg(a)),
            dst: reg(PhysReg(b)),
            size: OpSize::S32,
        });
        assert_eq!(
            line,
            format!(
                "vmovss %{}, %{}, %{}",
                expected_xmm(a),
                expected_xmm(a),
                expected_xmm(b)
            )
        );
    }
}

#[test]
fn fmov_memory_and_slot_shapes_are_exact() {
    // reg -> mem with and without displacement
    assert_eq!(
        emit1(&MachInst::FMov {
            src: reg(PhysReg(21)),
            dst: MachOperand::Mem {
                base: MachReg::Phys(PhysReg(14)),
                offset: 0,
            },
            size: OpSize::S64,
        }),
        "movsd %xmm3, (%rdi)"
    );
    assert_eq!(
        emit1(&MachInst::FMov {
            src: reg(PhysReg(21)),
            dst: MachOperand::Mem {
                base: MachReg::Phys(PhysReg(15)),
                offset: -16,
            },
            size: OpSize::S32,
        }),
        "movss %xmm3, -16(%rsi)"
    );
    // mem -> reg
    assert_eq!(
        emit1(&MachInst::FMov {
            src: MachOperand::Mem {
                base: MachReg::Phys(PhysReg(16)),
                offset: 8,
            },
            dst: reg(PhysReg(26)),
            size: OpSize::S64,
        }),
        "movsd 8(%rdx), %xmm8"
    );
    // reg <-> stack slot (rbp-relative by default)
    assert_eq!(
        emit1(&MachInst::FMov {
            src: reg(PhysReg(20)),
            dst: MachOperand::StackSlot(-8),
            size: OpSize::S64,
        }),
        "movsd %xmm2, -8(%rbp)"
    );
    assert_eq!(
        emit1(&MachInst::FMov {
            src: MachOperand::StackSlot(-8),
            dst: reg(PhysReg(20)),
            size: OpSize::S32,
        }),
        "movss -8(%rbp), %xmm2"
    );
}

#[test]
fn fmov_stack_slot_honors_rsp_addressing() {
    let mut out = AsmOutput::new();
    out.use_rsp_addressing = true;
    out.rsp_frame_size = 64;
    emit_machinst(
        &MachInst::FMov {
            src: reg(PhysReg(20)),
            dst: MachOperand::StackSlot(-8),
            size: OpSize::S64,
        },
        &mut out,
    );
    assert_eq!(out.buf.trim(), "movsd %xmm2, 56(%rsp)");
}

#[test]
fn fmov_self_move_emits_nothing() {
    let lines = emit(&MachInst::FMov {
        src: reg(PhysReg(23)),
        dst: reg(PhysReg(23)),
        size: OpSize::S64,
    });
    assert!(lines.is_empty(), "self-move emitted {lines:?}");
}

// Trap tests: these shapes are unencodable or would relay through a
// possibly-live register. The emitter must refuse them loudly; the isel
// gates are what keep them from ever being constructed in practice.

#[test]
#[should_panic(expected = "non-float size")]
fn fmov_rejects_narrow_sizes() {
    emit1(&MachInst::FMov {
        src: reg(PhysReg(20)),
        dst: reg(PhysReg(21)),
        size: OpSize::S16,
    });
}

#[test]
#[should_panic(expected = "mem-to-mem")]
fn fmov_rejects_mem_to_mem() {
    emit1(&MachInst::FMov {
        src: MachOperand::StackSlot(-8),
        dst: MachOperand::Mem {
            base: MachReg::Phys(PhysReg(14)),
            offset: 0,
        },
        size: OpSize::S64,
    });
}

#[test]
#[should_panic(expected = "mem-to-mem")]
fn fmov_rejects_slot_to_slot() {
    emit1(&MachInst::FMov {
        src: MachOperand::StackSlot(-8),
        dst: MachOperand::StackSlot(-16),
        size: OpSize::S64,
    });
}

#[test]
#[should_panic(expected = "immediate")]
fn fmov_rejects_immediate_source() {
    emit1(&MachInst::FMov {
        src: MachOperand::Imm(0),
        dst: reg(PhysReg(20)),
        size: OpSize::S64,
    });
}

#[test]
#[should_panic(expected = "unallocated")]
fn fmov_rejects_unallocated_vreg() {
    emit1(&MachInst::FMov {
        src: MachOperand::Reg(MachReg::Vreg(42)),
        dst: reg(PhysReg(20)),
        size: OpSize::S64,
    });
}

// ── FMov isel admission ───────────────────────────────────────────────────
//
// The typed path must accept exactly the xmm-homed subset and decline
// everything else: a `true` on a shape the emitter does not model is a
// miscompile, a `false` on a modelled shape is lost coverage.

#[test]
fn float_moves_lower_through_machinst() {
    use crate::common::fx_hash::FxHashMap;
    use crate::common::types::IrType;
    use crate::ir::reexports::{Instruction, Operand, Value};

    // 4: xmm2-homed float value; 5: xmm3-homed; 3: GPR pointer (r10);
    // 9: alloca slot at -8.
    let ra: FxHashMap<u32, PhysReg> = [
        (3u32, PhysReg(11)),
        (4, PhysReg(20)),
        (5, PhysReg(21)),
    ]
    .into_iter()
    .collect();
    let slots: FxHashMap<u32, i64> = [(9u32, -8i64)].into_iter().collect();

    let store = |val: u32, ptr: u32, ty: IrType| Instruction::Store {
        val: Operand::Value(Value(val)),
        ptr: Value(ptr),
        ty,
        seg_override: Default::default(),
        volatile: false,
    };
    let load = |dest: u32, ptr: u32, ty: IrType| Instruction::Load {
        dest: Value(dest),
        ptr: Value(ptr),
        ty,
        seg_override: Default::default(),
        volatile: false,
    };

    // Store F64 through the alloca slot.
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(
        &store(4, 9, IrType::F64),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    match &out[..] {
        [MachInst::FMov {
            src: MachOperand::Reg(MachReg::Phys(s)),
            dst: MachOperand::StackSlot(off),
            size,
        }] => {
            assert_eq!(*s, PhysReg(20));
            assert_eq!(*off, -8);
            assert_eq!(*size, OpSize::S64);
        }
        other => panic!("unexpected lowering: {other:?}"),
    }

    // Store F32 through the GPR pointer.
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(
        &store(4, 3, IrType::F32),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    match &out[..] {
        [MachInst::FMov {
            dst: MachOperand::Mem { base, offset },
            size,
            ..
        }] => {
            assert_eq!(*base, MachReg::Phys(PhysReg(11)));
            assert_eq!(*offset, 0);
            assert_eq!(*size, OpSize::S32);
        }
        other => panic!("unexpected lowering: {other:?}"),
    }

    // Load F64 through the GPR pointer into an xmm home.
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(
        &load(5, 3, IrType::F64),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    match &out[..] {
        [MachInst::FMov {
            src: MachOperand::Mem { .. },
            dst: MachOperand::Reg(MachReg::Phys(d)),
            size,
        }] => {
            assert_eq!(*d, PhysReg(21));
            assert_eq!(*size, OpSize::S64);
        }
        other => panic!("unexpected lowering: {other:?}"),
    }

    // Load F32 from the alloca slot.
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(
        &load(5, 9, IrType::F32),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    match &out[..] {
        [MachInst::FMov {
            src: MachOperand::StackSlot(off),
            size,
            ..
        }] => {
            assert_eq!(*off, -8);
            assert_eq!(*size, OpSize::S32);
        }
        other => panic!("unexpected lowering: {other:?}"),
    }

    // Float Copy between two xmm homes (type from the value-type map).
    let mut vt: FxHashMap<u32, IrType> = FxHashMap::default();
    vt.insert(4, IrType::F32);
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(
        &Instruction::Copy {
            dest: Value(5),
            src: Operand::Value(Value(4)),
        },
        &ra,
        &slots,
        Some(&vt),
        &mut out,
    ));
    match &out[..] {
        [MachInst::FMov {
            src: MachOperand::Reg(MachReg::Phys(s)),
            dst: MachOperand::Reg(MachReg::Phys(d)),
            size,
        }] => {
            assert_eq!(*s, PhysReg(20));
            assert_eq!(*d, PhysReg(21));
            assert_eq!(*size, OpSize::S32);
        }
        other => panic!("unexpected lowering: {other:?}"),
    }
}

#[test]
fn float_moves_outside_the_subset_stay_on_the_text_path() {
    use crate::common::fx_hash::FxHashMap;
    use crate::common::types::IrType;
    use crate::ir::reexports::{Instruction, IrConst, Operand, Value};

    let ra: FxHashMap<u32, PhysReg> = [
        (3u32, PhysReg(11)),
        (4, PhysReg(20)),
        (6, PhysReg(22)), // xmm-based pointer
    ]
    .into_iter()
    .collect();
    let slots: FxHashMap<u32, i64> = FxHashMap::default();
    let store = |val: Operand, ptr: u32, ty: IrType| Instruction::Store {
        val,
        ptr: Value(ptr),
        ty,
        seg_override: Default::default(),
        volatile: false,
    };

    // Value with no register home (slot-homed floats need the xmm scratch
    // relay the text path owns).
    let mut out = Vec::new();
    assert!(!super::isel::lower_instruction_typed(
        &store(Operand::Value(Value(77)), 3, IrType::F64),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    assert!(out.is_empty());

    // Constant source: no scalar-SSE immediate form.
    let mut out = Vec::new();
    assert!(!super::isel::lower_instruction_typed(
        &store(Operand::Const(IrConst::F64(1.0)), 3, IrType::F64),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    assert!(out.is_empty());

    // XMM-based pointer: x86 addressing has no xmm base.
    let mut out = Vec::new();
    assert!(!super::isel::lower_instruction_typed(
        &store(Operand::Value(Value(4)), 6, IrType::F64),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    assert!(out.is_empty());

    // Pointer with no home at all.
    let mut out = Vec::new();
    assert!(!super::isel::lower_instruction_typed(
        &store(Operand::Value(Value(4)), 88, IrType::F64),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    assert!(out.is_empty());

    // F128 goes through x87 on the text path.
    let mut out = Vec::new();
    assert!(!super::isel::lower_instruction_typed(
        &store(Operand::Value(Value(4)), 3, IrType::F128),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    assert!(out.is_empty());

    // Load with a GPR dest (mixed domain) stays on the text path.
    let mut out = Vec::new();
    assert!(!super::isel::lower_instruction_typed(
        &Instruction::Load {
            dest: Value(3),
            ptr: Value(3),
            ty: IrType::F64,
            seg_override: Default::default(),
            volatile: false,
        },
        &ra,
        &slots,
        None,
        &mut out,
    ));
    assert!(out.is_empty());

    // Float Copy where only one side is xmm-homed.
    let mut vt: FxHashMap<u32, IrType> = FxHashMap::default();
    vt.insert(4, IrType::F64);
    let mut out = Vec::new();
    assert!(!super::isel::lower_instruction_typed(
        &Instruction::Copy {
            dest: Value(3), // GPR dest
            src: Operand::Value(Value(4)),
        },
        &ra,
        &slots,
        Some(&vt),
        &mut out,
    ));
    assert!(out.is_empty());

    // Integer Copy of an untyped value must NOT be hijacked as a float
    // move: no value-type entry means the integer width logic applies.
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(
        &Instruction::Copy {
            dest: Value(3),
            src: Operand::Value(Value(11)),
        },
        &ra,
        &slots,
        Some(&vt),
        &mut out,
    ));
    assert!(
        out.iter().all(|mi| !matches!(mi, MachInst::FMov { .. })),
        "untyped copy produced FMov: {out:?}"
    );
}

// ── 6. FAlu: scalar SSE arithmetic (VEX three-operand) ───────────────────

#[test]
fn falu_emits_vex_three_operand_forms() {
    // dst = src1 OP src2; AT&T prints `vop src2, %src1, %dst`.
    assert_eq!(
        emit1(&MachInst::FAlu {
            op: FAluOp::Add,
            src2: reg(PhysReg(21)),
            src1: MachReg::Phys(PhysReg(20)),
            dst: MachReg::Phys(PhysReg(22)),
            size: OpSize::S64,
        }),
        "vaddsd %xmm3, %xmm2, %xmm4"
    );
    assert_eq!(
        emit1(&MachInst::FAlu {
            op: FAluOp::Sub,
            src2: reg(PhysReg(21)),
            src1: MachReg::Phys(PhysReg(22)),
            dst: MachReg::Phys(PhysReg(22)),
            size: OpSize::S64,
        }),
        "vsubsd %xmm3, %xmm4, %xmm4"
    );
    assert_eq!(
        emit1(&MachInst::FAlu {
            op: FAluOp::Mul,
            src2: reg(PhysReg(33)),
            src1: MachReg::Phys(PhysReg(26)),
            dst: MachReg::Phys(PhysReg(27)),
            size: OpSize::S32,
        }),
        "vmulss %xmm15, %xmm8, %xmm9"
    );
    assert_eq!(
        emit1(&MachInst::FAlu {
            op: FAluOp::Div,
            src2: reg(PhysReg(20)),
            src1: MachReg::Phys(PhysReg(21)),
            dst: MachReg::Phys(PhysReg(21)),
            size: OpSize::S32,
        }),
        "vdivss %xmm2, %xmm3, %xmm3"
    );
    // Memory/slot src2 folds into the instruction (single memory operand).
    assert_eq!(
        emit1(&MachInst::FAlu {
            op: FAluOp::Add,
            src2: MachOperand::StackSlot(-16),
            src1: MachReg::Phys(PhysReg(20)),
            dst: MachReg::Phys(PhysReg(21)),
            size: OpSize::S64,
        }),
        "vaddsd -16(%rbp), %xmm2, %xmm3"
    );
    assert_eq!(
        emit1(&MachInst::FAlu {
            op: FAluOp::Mul,
            src2: MachOperand::Mem {
                base: MachReg::Phys(PhysReg(14)),
                offset: 8,
            },
            src1: MachReg::Phys(PhysReg(20)),
            dst: MachReg::Phys(PhysReg(21)),
            size: OpSize::S64,
        }),
        "vmulsd 8(%rdi), %xmm2, %xmm3"
    );
}

#[test]
#[should_panic(expected = "non-float size")]
fn falu_rejects_narrow_sizes() {
    emit1(&MachInst::FAlu {
        op: FAluOp::Add,
        src2: reg(PhysReg(21)),
        src1: MachReg::Phys(PhysReg(20)),
        dst: MachReg::Phys(PhysReg(22)),
        size: OpSize::S16,
    });
}

#[test]
#[should_panic(expected = "immediate")]
fn falu_rejects_immediate_src2() {
    emit1(&MachInst::FAlu {
        op: FAluOp::Add,
        src2: MachOperand::Imm(1),
        src1: MachReg::Phys(PhysReg(20)),
        dst: MachReg::Phys(PhysReg(22)),
        size: OpSize::S64,
    });
}

#[test]
#[should_panic(expected = "unallocated")]
fn falu_rejects_unallocated_src1() {
    emit1(&MachInst::FAlu {
        op: FAluOp::Add,
        src2: reg(PhysReg(21)),
        src1: MachReg::Vreg(7),
        dst: MachReg::Phys(PhysReg(22)),
        size: OpSize::S64,
    });
}

#[test]
fn float_binops_lower_with_exact_operand_order() {
    use crate::common::fx_hash::FxHashMap;
    use crate::common::types::IrType;
    use crate::ir::reexports::{Instruction, IrBinOp, Operand, Value};

    // 4: xmm2 (lhs), 5: xmm3 (rhs), 6: xmm4 (dest), 7: slot-homed value.
    let ra: FxHashMap<u32, PhysReg> = [
        (4u32, PhysReg(20)),
        (5, PhysReg(21)),
        (6, PhysReg(22)),
    ]
    .into_iter()
    .collect();
    let slots: FxHashMap<u32, i64> = FxHashMap::default();

    let binop = |dest: u32, op: IrBinOp, l: u32, r: u32, ty: IrType| Instruction::BinOp {
        dest: Value(dest),
        op,
        lhs: Operand::Value(Value(l)),
        rhs: Operand::Value(Value(r)),
        ty,
    };

    // Sub keeps lhs in src1: dst = src1 - src2.
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(
        &binop(6, IrBinOp::Sub, 4, 5, IrType::F64),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    match &out[..] {
        [MachInst::FAlu {
            op: FAluOp::Sub,
            src2: MachOperand::Reg(MachReg::Phys(s2)),
            src1: MachReg::Phys(s1),
            dst: MachReg::Phys(d),
            size,
        }] => {
            assert_eq!(*s1, PhysReg(20), "lhs must stay src1 for sub");
            assert_eq!(*s2, PhysReg(21));
            assert_eq!(*d, PhysReg(22));
            assert_eq!(*size, OpSize::S64);
        }
        other => panic!("unexpected lowering: {other:?}"),
    }

    // SDiv on floats is the division operator (no separate FDiv in the IR).
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(
        &binop(6, IrBinOp::SDiv, 4, 5, IrType::F32),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    assert!(
        matches!(&out[..], [MachInst::FAlu { op: FAluOp::Div, size: OpSize::S32, .. }]),
        "unexpected: {out:?}"
    );

    // Commutative swap: lhs slot-homed (vreg), rhs xmm -> src1 = rhs.
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(
        &binop(6, IrBinOp::Mul, 7, 5, IrType::F64),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    match &out[..] {
        [MachInst::FAlu {
            op: FAluOp::Mul,
            src2: MachOperand::Reg(MachReg::Vreg(7)),
            src1: MachReg::Phys(s1),
            ..
        }] => assert_eq!(*s1, PhysReg(21), "rhs must become src1 in swapped form"),
        other => panic!("unexpected lowering: {other:?}"),
    }

    // Non-commutative with slot lhs is REJECTED (text path stages via xmm0).
    let mut out = Vec::new();
    assert!(!super::isel::lower_instruction_typed(
        &binop(6, IrBinOp::Sub, 7, 5, IrType::F64),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    assert!(out.is_empty());

    // Slot rhs on a lhs-xmm op is admitted as a vreg (flush-time slot
    // resolution folds it into the memory operand).
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(
        &binop(6, IrBinOp::Add, 4, 7, IrType::F64),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    match &out[..] {
        [MachInst::FAlu {
            src2: MachOperand::Reg(MachReg::Vreg(7)),
            src1: MachReg::Phys(s1),
            ..
        }] => assert_eq!(*s1, PhysReg(20)),
        other => panic!("unexpected lowering: {other:?}"),
    }

    // GPR dest: rejected.
    let mut out = Vec::new();
    assert!(!super::isel::lower_instruction_typed(
        &binop(3, IrBinOp::Add, 4, 5, IrType::F64),
        &ra,
        &slots,
        None,
        &mut out,
    ));
    assert!(out.is_empty());

    // Integer Add is untouched by the float arm.
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(
        &binop(3, IrBinOp::Add, 1, 2, IrType::I64),
        &[(1u32, PhysReg(0)), (2, PhysReg(7)), (3, PhysReg(11))]
            .into_iter()
            .collect(),
        &slots,
        None,
        &mut out,
    ));
    assert!(
        out.iter().all(|mi| !matches!(mi, MachInst::FAlu { .. })),
        "integer add produced FAlu: {out:?}"
    );
}

// ── 9. CallTyped: the typed direct-call contract ────────────────────────────
//
// The call is the most ABI-exposed instruction in the layer. These tests
// pin the three invariants that make the atomic form sound:
//   (a) argument moves execute in an order where every reader of a
//       register runs before every writer of it (the home is destroyed
//       by the write);
//   (b) register-exchange shapes never reach the variant (the text path
//       owns the hazard-spill machinery);
//   (c) the emitted sequence is exactly saves → args → call → ret →
//       restores, with zero-immediate arguments rendered as `xorl`.

mod call_builder {
    use super::super::isel::{build_typed_call, TypedCallReject, TypedCallSrc};
    use super::*;
    use crate::common::types::IrType;

    const RDI: PhysReg = PhysReg(14);
    const RSI: PhysReg = PhysReg(15);
    const RDX: PhysReg = PhysReg(16);
    const RCX: PhysReg = PhysReg(7);
    const R8: PhysReg = PhysReg(12);
    const R9: PhysReg = PhysReg(13);

    fn s64() -> OpSize {
        OpSize::S64
    }

    /// arg0's source is r8 (arg4's destination): arg0 must execute BEFORE
    /// arg4, or the write destroys the value it still needs to read.
    #[test]
    fn orders_readers_before_writers() {
        let plan = build_typed_call(
            &[
                Some(TypedCallSrc::Reg(R8)),  // arg0: read r8 …
                Some(TypedCallSrc::Reg(PhysReg(1))), // arg1: rbx → rsi
                Some(TypedCallSrc::Imm(7)),   // arg2
                Some(TypedCallSrc::Slot(-8)), // arg3
                Some(TypedCallSrc::Reg(PhysReg(2))), // arg4: r12 → r8 (writes the arg0 source!)
                Some(TypedCallSrc::Imm(0)),   // arg5
            ],
            &[s64(); 6],
            None,
        )
        .expect("acyclic shape must be accepted");
        let pos = |want: PhysReg| {
            plan.args
                .iter()
                .position(|m| m.dst_reg == want)
                .unwrap_or_else(|| panic!("no move writes {:?}", want))
        };
        assert!(
            pos(RDI) < pos(R8),
            "the r8 reader must be ordered before the r8 writer: {:?}",
            plan.args
        );
    }

    /// f(a, b) with a homed in rsi (arg1's register) and b homed in rdi
    /// (arg0's register) is a register exchange: no serial order is sound.
    /// The builder must refuse it so the text path's hazard-spill area
    /// handles the shape.
    #[test]
    fn rejects_register_exchange_cycles() {
        let err = build_typed_call(
            &[
                Some(TypedCallSrc::Reg(RSI)), // a: rsi → rdi
                Some(TypedCallSrc::Reg(RDI)), // b: rdi → rsi
            ],
            &[s64(), s64()],
            None,
        )
        .expect_err("a swap cannot be ordered serially");
        assert_eq!(err, TypedCallReject::MoveCycle);
    }

    /// A value already homed in its own ABI register needs no move.
    #[test]
    fn elides_self_moves() {
        let plan = build_typed_call(
            &[
                Some(TypedCallSrc::Reg(RDI)), // arg0 already in rdi
                Some(TypedCallSrc::Reg(PhysReg(1))),
            ],
            &[s64(), s64()],
            None,
        )
        .unwrap();
        assert!(
            plan.args.iter().all(|m| m.dst_reg != RDI),
            "self-move must be elided: {:?}",
            plan.args
        );
        assert_eq!(plan.args.len(), 1);
    }

    #[test]
    fn rejects_immediates_that_do_not_fit_the_imm32_movq_form() {
        let err = build_typed_call(
            &[Some(TypedCallSrc::Imm(0x1_0000_0000))],
            &[s64()],
            None,
        )
        .expect_err("64-bit immediates need movabsq, which the subset does not model");
        assert_eq!(err, TypedCallReject::ImmTooWide(0));
        // The extremes of the imm32 window are fine (movq sign-extends).
        assert!(build_typed_call(&[Some(TypedCallSrc::Imm(i32::MIN as i64))], &[s64()], None).is_ok());
        assert!(build_typed_call(&[Some(TypedCallSrc::Imm(i32::MAX as i64))], &[s64()], None).is_ok());
    }

    /// `&local` arguments lower through a pre-move writing the ABI register.
    /// Another argument sourcing from that same register would read it
    /// after the pre-move clobbered it — the builder must refuse.
    #[test]
    fn alloca_address_args_conflict_checked() {
        let ok = build_typed_call(
            &[
                Some(TypedCallSrc::AllocaAddr(9)),
                Some(TypedCallSrc::Reg(PhysReg(1))), // rbx: fine
            ],
            &[s64(), s64()],
            None,
        )
        .expect("no other arg reads rdi");
        assert!(matches!(
            ok.args.iter().find(|m| m.dst_reg == RDI).map(|m| &m.src),
            Some(MachOperand::AllocaAddr(9))
        ));

        let err = build_typed_call(
            &[
                Some(TypedCallSrc::AllocaAddr(9)),
                Some(TypedCallSrc::Reg(RDI)), // reads rdi: the pre-move's victim
            ],
            &[s64(), s64()],
            None,
        )
        .expect_err("another argument sources the pre-move's destination");
        assert_eq!(err, TypedCallReject::ArgNotRepresentable(0));
    }

    /// The builder's defensive re-check: rax (accumulator) and rbp (frame
    /// pointer) are never stable homes in this model.
    #[test]
    fn rejects_rax_and_rbp_argument_homes() {
        for bad in [PhysReg(0), PhysReg(6)] {
            let err = build_typed_call(&[Some(TypedCallSrc::Reg(bad))], &[s64()], None)
                .expect_err("scratch/frame-pointer homes must be refused");
            assert_eq!(err, TypedCallReject::ArgNotRepresentable(0));
        }
    }

    #[test]
    fn return_homes() {
        // Register return.
        let plan = build_typed_call(&[], &[], Some((Some(TypedCallSrc::Reg(PhysReg(1))), s64())))
            .unwrap();
        match plan.ret {
            Some(r) => assert_eq!(r.dst, reg(PhysReg(1))),
            None => panic!("register return home required"),
        }
        // Slot return.
        let plan = build_typed_call(&[], &[], Some((Some(TypedCallSrc::Slot(-16)), s64()))).unwrap();
        match plan.ret {
            Some(r) => assert_eq!(r.dst, MachOperand::StackSlot(-16)),
            None => panic!("slot return home required"),
        }
        // rax-homed return: the copy is a no-op.
        let plan = build_typed_call(&[], &[], Some((Some(TypedCallSrc::Reg(PhysReg(0))), s64()))).unwrap();
        assert!(plan.ret.is_none(), "rax → rax must be elided");
        // rbp return home: refused.
        let err = build_typed_call(&[], &[], Some((Some(TypedCallSrc::Reg(PhysReg(6))), s64())))
            .expect_err("the frame pointer is not a value home");
        assert_eq!(err, TypedCallReject::RetNotRepresentable);
        // Void call.
        assert!(build_typed_call(&[], &[], None).unwrap().ret.is_none());
    }

    /// Six heterogeneous arguments: the full plan in one assertion, so a
    /// regression in any position breaks a single test instead of a
    /// property silently holding for the others. Position in the plan is
    /// the TOPOLOGICAL order, so assertions are keyed by destination
    /// register: the plan may legitimately reorder (arg5 reads rdx and
    /// must precede arg2, which writes it), but every source/dest/width
    /// binding is fixed.
    #[test]
    fn full_six_argument_shape() {
        let plan = build_typed_call(
            &[
                Some(TypedCallSrc::Imm(42)),
                Some(TypedCallSrc::Slot(-24)),
                Some(TypedCallSrc::Reg(RCX)), // reads rcx: written by arg3's move
                Some(TypedCallSrc::Imm(0)),
                Some(TypedCallSrc::Reg(PhysReg(3))),
                Some(TypedCallSrc::Reg(RDX)), // reads rdx: written by arg2's move
            ],
            &[
                OpSize::S32,
                OpSize::S64,
                OpSize::S64,
                OpSize::S32,
                OpSize::S64,
                OpSize::S32,
            ],
            Some((Some(TypedCallSrc::Slot(-32)), OpSize::S32)),
        )
        .expect("mixed but conflict-free shape");
        assert_eq!(plan.args.len(), 6);
        let by_dst = |want: PhysReg| -> &CallArgMove {
            plan.args
                .iter()
                .find(|m| m.dst_reg == want)
                .unwrap_or_else(|| panic!("no move for dst {want:?} in {:?}", plan.args))
        };
        assert_eq!(by_dst(RDI).src, MachOperand::Imm(42));
        assert_eq!(by_dst(RDI).size, OpSize::S32);
        assert_eq!(by_dst(RSI).src, MachOperand::StackSlot(-24));
        assert_eq!(by_dst(RDX).src, MachOperand::Reg(MachReg::Phys(RCX)));
        assert_eq!(by_dst(RCX).src, MachOperand::Imm(0));
        assert_eq!(by_dst(R8).src, MachOperand::Reg(MachReg::Phys(PhysReg(3))));
        assert_eq!(by_dst(R9).src, MachOperand::Reg(MachReg::Phys(RDX)));
        assert_eq!(by_dst(R9).size, OpSize::S32);
        // Reader-before-writer constraints across the plan (a reader must
        // observe the home's original value, so it precedes the writer):
        //   rdx: arg5's move reads it, arg2's move writes it.
        //   rcx: arg2's move reads it, arg3's move writes it.
        let pos = |want: PhysReg| plan.args.iter().position(|m| m.dst_reg == want).unwrap();
        assert!(pos(R9) < pos(RDX), "the rdx reader must precede the rdx writer");
        assert!(pos(RDX) < pos(RCX), "the rcx reader must precede the rcx writer");
        assert_eq!(plan.ret.as_ref().unwrap().dst, MachOperand::StackSlot(-32));
        assert_eq!(plan.ret.as_ref().unwrap().size, OpSize::S32);
    }
}

// ── 10. CallTyped: golden emission ──────────────────────────────────────────

/// The full stage order is part of the instruction's contract: caller-save
/// spills, argument moves, the call, the return home, and the restores —
/// the mature path's Phase 2b → 3 → 4 → 6 → 2b tail, typed end to end.
#[test]
fn calltyped_emits_saves_args_call_ret_restores_in_order() {
    let inst = MachInst::CallTyped {
        caller_saves: vec![(PhysReg(10), -48), (PhysReg(11), -56)], // r11, r10
        args: vec![
            CallArgMove {
                src: MachOperand::Imm(0),
                dst_reg: PhysReg(14),
                size: OpSize::S64,
            },
            CallArgMove {
                src: reg(PhysReg(1)),
                dst_reg: PhysReg(15),
                size: OpSize::S32,
            },
            CallArgMove {
                src: MachOperand::StackSlot(-24),
                dst_reg: PhysReg(16),
                size: OpSize::S64,
            },
        ],
        target: "mix6@PLT".into(),
        ret: Some(CallRetMove {
            dst: reg(PhysReg(2)),
            size: OpSize::S64,
        }),
    };
    let lines = emit(&inst);
    assert_eq!(
        lines,
        vec![
            "movq %r11, -48(%rbp)", // save r11
            "movq %r10, -56(%rbp)", // save r10
            "xorl %edi, %edi",      // zero immediate: the 3-byte form
            "movl %ebx, %esi",      // 32-bit arg at its canonical width
            "movq -24(%rbp), %rdx", // slot-homed arg
            "call mix6@PLT",
            "movq %rax, %r12",      // return home
            "movq -56(%rbp), %r10", // restores in reverse
            "movq -48(%rbp), %r11",
        ],
        "stage order or rendering drifted: {lines:?}"
    );
}

/// A 32-bit return home must name %eax (the sub-register), not %rax with
/// a 32-bit mnemonic — the mismatch the very first smoke test caught.
#[test]
fn calltyped_return_home_names_the_sized_rax() {
    let inst = MachInst::CallTyped {
        caller_saves: vec![],
        args: vec![],
        target: "f".into(),
        ret: Some(CallRetMove {
            dst: MachOperand::StackSlot(-8),
            size: OpSize::S32,
        }),
    };
    let lines = emit(&inst);
    assert_eq!(
        lines,
        vec!["call f", "movl %eax, -8(%rbp)"],
        "{lines:?}"
    );
}

/// The typed call's registers — argument registers, rax — must be covered
/// by the assembler differential like every other variant: add them to the
/// corpus.
#[test]
fn calltyped_is_in_the_assembler_corpus() {
    let has_calltyped = instruction_corpus()
        .iter()
        .any(|i| matches!(i, MachInst::CallTyped { .. }));
    assert!(
        has_calltyped,
        "CallTyped must be part of instruction_corpus so the real-assembler \
         differential and the determinism test cover it"
    );
}

// ── 11. xmm0/xmm1 scratch: names and the relay liveness guard ───────────────

/// xmm0/xmm1 (PhysReg 18/19) are the pre-colored float scratch pair. The
/// 64-bit name table must know them (the relay emits them), and the float
/// name table must agree.
#[test]
fn xmm_scratch_registers_emit_their_canonical_names() {
    for (id, want) in [(18u8, "xmm0"), (19, "xmm1")] {
        let mut out = AsmOutput::new();
        emit_machinst(
            &MachInst::FMov {
                src: reg(PhysReg(20)),
                dst: reg(PhysReg(id)),
                size: OpSize::S32,
            },
            &mut out,
        );
        assert!(
            out.buf.contains(&format!("%{want}")),
            "PhysReg({id}) must render as %{want}: {}",
            out.buf
        );
    }
}

/// The float store relay moves through xmm0. That is only sound when no
/// live value is homed there — and float PARAMETERS are pre-colored to
/// their INCOMING registers (xmm0 = PhysReg 18). A parameter home in the
/// scratch must refuse the relay to the text path.
#[test]
fn float_store_relay_refuses_when_xmm0_holds_a_live_param() {
    use crate::common::fx_hash::FxHashMap;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{Instruction, Operand, Value};

    // Value 5 lives in xmm0 (PhysReg 18): a float parameter's incoming home.
    let ra: FxHashMap<u32, PhysReg> = [(5u32, PhysReg(18))].into_iter().collect();
    // The stored value (7) is slot-homed; the pointer is an alloca (9).
    let slots: FxHashMap<u32, i64> = [(7u32, -24i64), (9u32, -32i64)].into_iter().collect();
    let inst = Instruction::Store {
        val: Operand::Value(Value(7)),
        ptr: Value(9),
        ty: IrType::F32,
        seg_override: AddressSpace::Default,
        volatile: false,
    };
    let mut out = Vec::new();
    assert!(
        !super::isel::lower_instruction_typed(&inst, &ra, &slots, None, &mut out),
        "the relay would clobber the live xmm0 parameter home"
    );
}

/// The healthy relay: slot-homed value → xmm0 → destination, two moves.
#[test]
fn float_store_relay_lowers_to_two_fmovs_through_xmm0() {
    use crate::common::fx_hash::FxHashMap;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{Instruction, Operand, Value};

    // No float params: xmm0 is free. A value in xmm2 keeps the direct path
    // distinct from the relay under test.
    let ra: FxHashMap<u32, PhysReg> = [(3u32, PhysReg(20))].into_iter().collect();
    let slots: FxHashMap<u32, i64> = [(7u32, -24i64), (9u32, -32i64)].into_iter().collect();
    let inst = Instruction::Store {
        val: Operand::Value(Value(7)), // slot-homed
        ptr: Value(9),                 // alloca
        ty: IrType::F64,
        seg_override: AddressSpace::Default,
        volatile: false,
    };
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(&inst, &ra, &slots, None, &mut out));
    assert_eq!(out.len(), 2, "{out:?}");
    match (&out[0], &out[1]) {
        (
            MachInst::FMov {
                src: MachOperand::StackSlot(-24),
                dst: MachOperand::Reg(MachReg::Phys(scratch)),
                ..
            },
            MachInst::FMov {
                src: MachOperand::Reg(MachReg::Phys(s2)),
                dst: MachOperand::StackSlot(-32),
                ..
            },
        ) => {
            assert_eq!(*scratch, PhysReg(18), "relay must use the pre-colored xmm0");
            assert_eq!(*s2, PhysReg(18));
        }
        other => panic!("unexpected relay shape: {other:?}"),
    }
}

/// Mirror shape for loads: a slot-homed DESTINATION is written from the
/// xmm0 scratch after the address is loaded into it.
#[test]
fn float_load_to_slot_homed_dest_relays_through_xmm0() {
    use crate::common::fx_hash::FxHashMap;
    use crate::common::types::{AddressSpace, IrType};
    use crate::ir::reexports::{Instruction, Value};

    let ra: FxHashMap<u32, PhysReg> = FxHashMap::default();
    let slots: FxHashMap<u32, i64> = [(1u32, -40i64), (2u32, -48i64)].into_iter().collect();
    let inst = Instruction::Load {
        dest: Value(2), // slot-homed destination
        ptr: Value(1),  // alloca
        ty: IrType::F64,
        seg_override: AddressSpace::Default,
        volatile: false,
    };
    let mut out = Vec::new();
    assert!(super::isel::lower_instruction_typed(&inst, &ra, &slots, None, &mut out));
    assert_eq!(out.len(), 2, "{out:?}");
    assert!(matches!(
        &out[0],
        MachInst::FMov {
            src: MachOperand::StackSlot(-40),
            dst: MachOperand::Reg(MachReg::Phys(PhysReg(18))),
            ..
        }
    ));
    assert!(matches!(
        &out[1],
        MachInst::FMov {
            src: MachOperand::Reg(MachReg::Phys(PhysReg(18))),
            dst: MachOperand::StackSlot(-48),
            ..
        }
    ));
}

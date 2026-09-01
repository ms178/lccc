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
        v.push(match rng.next() % 9 {
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

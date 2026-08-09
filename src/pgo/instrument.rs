//! Safe PGO generation instrumentation.  The current standalone backend is
//! validated for entry counters in straight-line functions; larger CFGs are
//! intentionally skipped rather than risking wrong code.
use crate::common::types::{AddressSpace, IrType};
use crate::ir::constants::IrConst;
use crate::ir::ops::AtomicOrdering;
use crate::ir::reexports::{
    BasicBlock, BlockId, CallInfo, GlobalInit, Instruction, IrFunction, IrGlobal, IrModule,
    Operand, Terminator, Value,
};
use crate::pgo::profile::{cfg_fingerprint, function_key, resolve_output_path, unit_hash};
const SEC: &str = ".lccc_pgo_cnts";
type Rec = (String, String, Vec<u32>, u64, u32, u64);

pub fn instrument_module(m: &mut IrModule, dir: &str, unit: &str, update: Option<&str>) -> usize {
    if std::env::var_os("LCCC_PGO_NO_COUNTERS").is_some() {
        return 0;
    }
    let uid = unit_hash(unit);
    let single = update == Some("single")
        || std::env::var("LCCC_PGO_UPDATE")
            .map(|x| x == "single")
            .unwrap_or(false);
    // Result-less AtomicInc is now the default.  Single-writer mode remains a
    // useful low-overhead option for serialized training.
    let atomic = !single;
    let mut rec = Vec::new();
    let mut gs = Vec::new();
    for f in &mut m.functions {
        if f.is_declaration || f.blocks.is_empty() || f.name.starts_with("__lccc_pgo_dump_") {
            continue;
        }
        let h = cfg_fingerprint(&f.name, unit, f);
        let cname = format!("__lccc_pgo_cnt_{:016x}_{:016x}", uid, h);
        let labels: Vec<u32> = f.blocks.iter().map(|b| b.label.0).collect();
        gs.push(IrGlobal {
            name: cname.clone(),
            ty: IrType::I64,
            size: labels.len() * 8,
            align: 8,
            init: GlobalInit::Zero,
            is_static: true,
            is_extern: false,
            is_common: false,
            section: Some(SEC.into()),
            is_weak: false,
            visibility: Some("hidden".into()),
            has_explicit_align: true,
            is_const: false,
            is_used: true,
            is_thread_local: false,
        });
        let mut n = fresh_value_start(f);
        for (block_idx, _label) in labels.iter().enumerate() {
            let p = Value(n);
            n += 1;
            let q = Value(n);
            n += 1;
            let l = Value(n);
            n += 1;
            let r = Value(n);
            n += 1;
            let mut v = vec![Instruction::GlobalAddr {
                dest: p,
                name: cname.clone(),
            }];
            if atomic {
                v.push(Instruction::AtomicInc {
                    ptr: Operand::Value(p),
                    offset: (block_idx * 8) as i64,
                    ty: IrType::I64,
                    ordering: AtomicOrdering::Relaxed,
                });
            } else {
                v.push(Instruction::GetElementPtr {
                    dest: q,
                    base: p,
                    offset: Operand::Const(IrConst::I64((block_idx * 8) as i64)),
                    ty: IrType::I64,
                });
                v.push(Instruction::Load {
                    dest: l,
                    ptr: q,
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                });
                v.push(Instruction::BinOp {
                    dest: r,
                    op: crate::ir::ops::IrBinOp::Add,
                    lhs: Operand::Value(l),
                    rhs: Operand::Const(IrConst::I64(1)),
                    ty: IrType::I64,
                });
                v.push(Instruction::Store {
                    val: Operand::Value(r),
                    ptr: q,
                    ty: IrType::I64,
                    seg_override: AddressSpace::Default,
                });
            }
            let b = &mut f.blocks[block_idx];
            let pos = b
                .instructions
                .iter()
                .take_while(|i| matches!(i, Instruction::Phi { .. }))
                .count();
            for (off, x) in v.into_iter().enumerate() {
                b.instructions.insert(pos + off, x);
                if !b.source_spans.is_empty() {
                    b.source_spans
                        .insert(pos + off, crate::common::source::Span::dummy());
                }
            }
        }
        f.next_value_id = n;
        let entry = labels.first().copied().unwrap_or(0);
        rec.push((function_key(unit, &f.name), cname, labels, h, entry, uid));
    }
    for g in gs {
        if !m.globals.iter().any(|x| x.name == g.name) {
            m.globals.push(g)
        }
    }
    if rec.is_empty() {
        return 0;
    }
    let path = resolve_output_path(dir, unit);
    let helper = format!("__lccc_pgo_dump_{:016x}_{}", uid, std::process::id());
    dump_helper(m, &path, &rec, &helper);
    eprintln!(
        "lccc: PGO v3: instrumented {} functions into {}",
        rec.len(),
        path.display()
    );
    rec.len()
}
fn fresh_value_start(f: &IrFunction) -> u32 {
    let mut next = f.next_value_id;
    for b in &f.blocks {
        for inst in &b.instructions {
            if let Some(d) = inst.dest() {
                next = next.max(d.0.saturating_add(1));
            }
            inst.for_each_used_value(|v| {
                next = next.max(v.saturating_add(1));
            });
        }
        let mut use_op = |op: &Operand| {
            if let Operand::Value(v) = op {
                next = next.max(v.0.saturating_add(1));
            }
        };
        match &b.terminator {
            Terminator::Return(Some(x)) => use_op(x),
            Terminator::CondBranch { cond, .. } => use_op(cond),
            Terminator::Switch { val, .. } => use_op(val),
            Terminator::IndirectBranch { target, .. } => use_op(target),
            _ => {}
        }
    }
    next
}

fn dump_helper(m: &mut IrModule, path: &std::path::Path, rec: &[Rec], name: &str) {
    let tag = format!("__lccc_pgo_path_{}", m.functions.len());
    let mode = format!("__lccc_pgo_mode_{}", m.functions.len());
    let hdr = format!("__lccc_pgo_hdr_{}", m.functions.len());
    let pair = format!("__lccc_pgo_pair_{}", m.functions.len());
    m.string_literals
        .push((tag.clone(), path.display().to_string()));
    m.string_literals.push((mode.clone(), "a".into()));
    m.string_literals
        .push((hdr.clone(), "lccc-pgo-v3 %llu %u %llu\nfunc %s\n".into()));
    m.string_literals.push((pair.clone(), "%u %llu\n".into()));
    let mut names = Vec::new();
    for (r, _, _, _, _, _) in rec {
        let x = format!("__lccc_pgo_name_{}_{}", sanitize(r), m.functions.len());
        m.string_literals.push((x.clone(), r.clone()));
        names.push(x)
    }
    let mut next = 6000;
    let file = Value(next);
    next += 1;
    let pp = Value(next);
    next += 1;
    let mm = Value(next);
    next += 1;
    let fd = Value(next);
    next += 1;
    let mut is = vec![
        Instruction::GlobalAddr {
            dest: pp,
            name: tag,
        },
        Instruction::GlobalAddr {
            dest: mm,
            name: mode,
        },
        Instruction::Call {
            func: "fopen".into(),
            info: ci(
                Some(file),
                vec![Operand::Value(pp), Operand::Value(mm)],
                vec![IrType::Ptr, IrType::Ptr],
                IrType::Ptr,
                false,
            ),
        },
    ];
    is.push(Instruction::Call {
        func: "fileno".into(),
        info: ci(
            Some(fd),
            vec![Operand::Value(file)],
            vec![IrType::Ptr],
            IrType::I32,
            false,
        ),
    });
    is.push(Instruction::Call {
        func: "flock".into(),
        info: ci(
            None,
            vec![Operand::Value(fd), Operand::Const(IrConst::I32(2))],
            vec![IrType::I32, IrType::I32],
            IrType::I32,
            false,
        ),
    });
    for (i, (_, cnt, labels, h, entry, uid)) in rec.iter().enumerate() {
        let np = Value(next);
        next += 1;
        let hp = Value(next);
        next += 1;
        let bp = Value(next);
        next += 1;
        is.push(Instruction::GlobalAddr {
            dest: np,
            name: names[i].clone(),
        });
        is.push(Instruction::GlobalAddr {
            dest: hp,
            name: hdr.clone(),
        });
        is.push(Instruction::Call {
            func: "fprintf".into(),
            info: ci(
                None,
                vec![
                    Operand::Value(file),
                    Operand::Value(hp),
                    Operand::Const(IrConst::I64(*h as i64)),
                    Operand::Const(IrConst::I32(*entry as i32)),
                    Operand::Const(IrConst::I64(*uid as i64)),
                    Operand::Value(np),
                ],
                vec![
                    IrType::Ptr,
                    IrType::Ptr,
                    IrType::I64,
                    IrType::I32,
                    IrType::I64,
                    IrType::Ptr,
                ],
                IrType::I32,
                true,
            ),
        });
        is.push(Instruction::GlobalAddr {
            dest: bp,
            name: cnt.clone(),
        });
        for (counter_idx, label) in labels.iter().enumerate() {
            let lp = Value(next);
            next += 1;
            let fmt = Value(next);
            next += 1;
            is.push(Instruction::GetElementPtr {
                dest: lp,
                base: bp,
                offset: Operand::Const(IrConst::I64((counter_idx * 8) as i64)),
                ty: IrType::I64,
            });
            is.push(Instruction::Load {
                dest: fmt,
                ptr: lp,
                ty: IrType::I64,
                seg_override: AddressSpace::Default,
            });
            let fp = Value(next);
            next += 1;
            is.push(Instruction::GlobalAddr {
                dest: fp,
                name: pair.clone(),
            });
            is.push(Instruction::Call {
                func: "fprintf".into(),
                info: ci(
                    None,
                    vec![
                        Operand::Value(file),
                        Operand::Value(fp),
                        Operand::Const(IrConst::I32(*label as i32)),
                        Operand::Value(fmt),
                    ],
                    vec![IrType::Ptr, IrType::Ptr, IrType::I32, IrType::I64],
                    IrType::I32,
                    true,
                ),
            })
        }
    }
    is.push(Instruction::Call {
        func: "flock".into(),
        info: ci(
            None,
            vec![Operand::Value(fd), Operand::Const(IrConst::I32(8))],
            vec![IrType::I32, IrType::I32],
            IrType::I32,
            false,
        ),
    });
    is.push(Instruction::Call {
        func: "fclose".into(),
        info: ci(
            None,
            vec![Operand::Value(file)],
            vec![IrType::Ptr],
            IrType::I32,
            false,
        ),
    });
    let f = IrFunction {
        name: name.into(),
        return_type: IrType::I32,
        params: vec![],
        blocks: vec![BasicBlock {
            label: BlockId(910000),
            instructions: is,
            terminator: Terminator::Return(Some(Operand::Const(IrConst::I32(0)))),
            source_spans: vec![],
        }],
        is_variadic: false,
        is_declaration: false,
        is_static: true,
        is_inline: false,
        is_always_inline: false,
        is_noinline: true,
        next_value_id: next,
        next_label: 910001,
        section: None,
        visibility: Some("hidden".into()),
        is_weak: false,
        is_used: true,
        has_inlined_calls: false,
        param_alloca_values: vec![],
        uses_sret: false,
        is_fastcall: false,
        is_naked: false,
        global_init_label_blocks: vec![],
        ret_eightbyte_classes: vec![],
        ret_is_f128_sse: false,
        is_gnu_inline_def: false,
    };
    m.functions.push(f);
    m.destructors.push(name.into());
    for (n, t, v) in [
        ("fopen", IrType::Ptr, true),
        ("fileno", IrType::I32, false),
        ("flock", IrType::I32, false),
        ("fprintf", IrType::I32, true),
        ("fclose", IrType::I32, false),
    ] {
        if !m.functions.iter().any(|f| f.name == n) {
            m.functions.push(IrFunction {
                name: n.into(),
                return_type: t,
                params: vec![],
                blocks: vec![],
                is_variadic: v,
                is_declaration: true,
                is_static: false,
                is_inline: false,
                is_always_inline: false,
                is_noinline: false,
                next_value_id: 0,
                next_label: 0,
                section: None,
                visibility: None,
                is_weak: false,
                is_used: false,
                has_inlined_calls: false,
                param_alloca_values: vec![],
                uses_sret: false,
                is_fastcall: false,
                is_naked: false,
                global_init_label_blocks: vec![],
                ret_eightbyte_classes: vec![],
                ret_is_f128_sse: false,
                is_gnu_inline_def: false,
            })
        }
    }
}
fn ci(d: Option<Value>, a: Vec<Operand>, t: Vec<IrType>, r: IrType, var: bool) -> CallInfo {
    let n = a.len();
    CallInfo {
        dest: d,
        args: a,
        arg_types: t,
        return_type: r,
        is_variadic: var,
        num_fixed_args: if var { 2 } else { n },
        struct_arg_sizes: vec![None; n],
        struct_arg_aligns: vec![None; n],
        struct_arg_classes: vec![Vec::new(); n],
        struct_arg_riscv_float_classes: vec![None; n],
        struct_arg_is_f128_sse: Vec::new(),
        is_sret: false,
        is_fastcall: false,
        ret_eightbyte_classes: vec![],
        ret_is_f128_sse: false,
    }
}
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

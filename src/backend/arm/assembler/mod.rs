//! Native AArch64 assembler.
//!
//! Parses `.s` assembly text (as emitted by the AArch64 codegen) and produces
//! ELF `.o` object files, removing the dependency on `aarch64-linux-gnu-gcc`
//! for assembly.
//!
//! Architecture:
//! - `parser.rs`     – Tokenize + parse assembly text into `AsmStatement` items
//! - `encoder.rs`    – Encode AArch64 instructions into 32-bit machine words
//! - `elf_writer.rs` – Write ELF object files with sections, symbols, and relocations

pub(crate) mod cfi;
pub mod elf_writer;
pub mod encoder;
pub mod parser;

use crate::common::fx_hash::FxHashMap;
use elf_writer::ElfWriter;
use parser::{AsmDirective, AsmStatement, DataValue, Operand, parse_asm};

/// Assemble AArch64 assembly text into an ELF object file.
///
/// This is the default assembler (used when the `gcc_assembler` feature is disabled).
pub fn assemble(asm_text: &str, output_path: &str) -> Result<(), String> {
    let statements = prepare_statements(asm_text)?;
    let mut writer = ElfWriter::new();
    writer.process_statements(&statements)?;
    writer.write_elf(output_path)?;
    Ok(())
}

/// Parse and run the statement-level passes ahead of the ELF writer:
/// literal-pool expansion, then numeric-label renaming (`1:`/`1f`).
/// Shared with the writer's unit tests so they assemble exactly what
/// [`assemble`] would.
pub(super) fn prepare_statements(asm_text: &str) -> Result<Vec<AsmStatement>, String> {
    let statements = parse_asm(asm_text)?;
    let statements = expand_literal_pools(&statements);
    Ok(resolve_numeric_labels(&statements))
}

/// Expand `LdrLiteralPool` pseudo-instructions into literal pool loads.
///
/// `ldr Xn, =symbol` must load the exact linker-resolved address via a literal
/// pool, not via `adrp+add` (which gives a PC-relative address). This matters
/// for code that runs at a different address than its linked address (e.g.,
/// early boot code running at physical addresses before MMU setup).
///
/// This pass replaces each `LdrLiteralPool` with a `ldr Xn, .Llpool_N`
/// instruction and accumulates pool entries. Pool entries are flushed (emitted
/// as `.quad` data with labels) when:
/// - A `.ltorg` / `.pool` directive is encountered
/// - A section-changing directive is encountered (`.section`, `.pushsection`, `.text`, `.data`)
/// - The end of the statement list is reached
///
/// Each pool entry is 8-byte aligned and consists of:
///   .Llpool_N: .quad symbol[+addend]
fn expand_literal_pools(statements: &[AsmStatement]) -> Vec<AsmStatement> {
    struct PoolEntry {
        label: String,
        symbol: String,
        addend: i64,
    }

    let mut result = Vec::with_capacity(statements.len());
    let mut pending_pool: Vec<PoolEntry> = Vec::new();
    let mut pool_counter: usize = 0;

    // TODO: Track W vs X register to emit .long (4-byte) entries for W registers
    // instead of always emitting .quad (8-byte). Currently safe on little-endian
    // (loads low 32 bits) but wrong on big-endian and wastes 4 bytes per W entry.
    let flush_pool = |pool: &mut Vec<PoolEntry>, out: &mut Vec<AsmStatement>| {
        if pool.is_empty() {
            return;
        }
        // Align pool to 8 bytes
        out.push(AsmStatement::Directive(AsmDirective::Balign(8)));
        for entry in pool.drain(..) {
            out.push(AsmStatement::Label(entry.label));
            let value = if entry.addend != 0 {
                format!("{} + ({})", entry.symbol, entry.addend)
            } else {
                entry.symbol
            };
            out.push(AsmStatement::Directive(AsmDirective::Quad(vec![
                DataValue::Expr(value),
            ])));
        }
    };

    for stmt in statements {
        match stmt {
            AsmStatement::LdrLiteralPool {
                reg,
                symbol,
                addend,
            } => {
                let pool_label = format!(".Llpool_{}", pool_counter);
                pool_counter += 1;

                // Emit: ldr reg, .Llpool_N (PC-relative literal load)
                result.push(AsmStatement::Instruction {
                    mnemonic: "ldr".to_string(),
                    operands: vec![
                        Operand::Reg(reg.clone()),
                        Operand::Symbol(pool_label.clone()),
                    ],
                    raw_operands: format!("{}, {}", reg, pool_label),
                });

                pending_pool.push(PoolEntry {
                    label: pool_label,
                    symbol: symbol.clone(),
                    addend: *addend,
                });
            }
            AsmStatement::Directive(AsmDirective::Ltorg) => {
                flush_pool(&mut pending_pool, &mut result);
            }
            AsmStatement::Directive(AsmDirective::Section(_))
            | AsmStatement::Directive(AsmDirective::PushSection(_))
            | AsmStatement::Directive(AsmDirective::PopSection)
            | AsmStatement::Directive(AsmDirective::Previous) => {
                // Flush pool before section change (pool must be in the same section)
                flush_pool(&mut pending_pool, &mut result);
                result.push(stmt.clone());
            }
            _ => {
                result.push(stmt.clone());
            }
        }
    }

    // Flush any remaining pool entries at the end
    flush_pool(&mut pending_pool, &mut result);

    result
}

/// Check if a label name is a GNU assembler numeric label (e.g., "1", "42").
fn is_numeric_label(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())
}

/// Check if a string is a numeric forward/backward reference like "1f" or "2b".
/// Returns Some((number_str, is_forward)) if it is, None otherwise.
fn parse_numeric_ref(name: &str) -> Option<(&str, bool)> {
    if name.len() < 2 {
        return None;
    }
    let last = name.as_bytes()[name.len() - 1];
    let num_part = &name[..name.len() - 1];
    if !num_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match last {
        b'f' | b'F' => Some((num_part, true)),
        b'b' | b'B' => Some((num_part, false)),
        _ => None,
    }
}

/// Resolve a numeric label reference to its unique name.
fn resolve_numeric_name(
    name: &str,
    current_idx: usize,
    defs: &FxHashMap<String, Vec<(usize, String)>>,
) -> Option<String> {
    let (num, is_forward) = parse_numeric_ref(name)?;
    let def_list = defs.get(num)?;

    if is_forward {
        def_list
            .iter()
            .find(|(idx, _)| *idx > current_idx)
            .map(|(_, name)| name.clone())
    } else {
        def_list
            .iter()
            .rev()
            .find(|(idx, _)| *idx < current_idx)
            .map(|(_, name)| name.clone())
    }
}

/// Resolve numeric local labels (1:, 2:, etc.) and their references (1f, 1b)
/// into unique internal label names.
///
/// GNU assembler numeric labels can be defined multiple times. Each forward
/// reference `Nf` refers to the next definition of `N`, and each backward
/// reference `Nb` refers to the most recent definition of `N`.
fn resolve_numeric_labels(statements: &[AsmStatement]) -> Vec<AsmStatement> {
    // First pass: find all numeric label definitions and assign unique names.
    let mut defs: FxHashMap<String, Vec<(usize, String)>> = FxHashMap::default();
    let mut unique_counter: FxHashMap<String, usize> = FxHashMap::default();

    for (i, stmt) in statements.iter().enumerate() {
        if let AsmStatement::Label(name) = stmt {
            if is_numeric_label(name) {
                let count = unique_counter.entry(name.clone()).or_insert(0);
                let unique_name = format!(".Lnum_{}_{}", name, *count);
                *count += 1;
                defs.entry(name.clone()).or_default().push((i, unique_name));
            }
        }
    }

    // If no numeric labels found, return original
    if defs.is_empty() {
        return statements.to_vec();
    }

    // Second pass: resolve all references
    let mut result = Vec::with_capacity(statements.len());
    for (i, stmt) in statements.iter().enumerate() {
        match stmt {
            AsmStatement::Label(name) if is_numeric_label(name) => {
                if let Some(def_list) = defs.get(name) {
                    if let Some((_, unique_name)) = def_list.iter().find(|(idx, _)| *idx == i) {
                        result.push(AsmStatement::Label(unique_name.clone()));
                        continue;
                    }
                }
                result.push(stmt.clone());
            }
            AsmStatement::Instruction {
                mnemonic,
                operands,
                raw_operands,
            } => {
                let new_ops: Vec<Operand> = operands
                    .iter()
                    .map(|op| resolve_numeric_operand(op, i, &defs))
                    .collect();
                result.push(AsmStatement::Instruction {
                    mnemonic: mnemonic.clone(),
                    operands: new_ops,
                    raw_operands: raw_operands.clone(),
                });
            }
            AsmStatement::Directive(dir) => {
                result.push(AsmStatement::Directive(resolve_numeric_directive(
                    dir, i, &defs,
                )));
            }
            _ => result.push(stmt.clone()),
        }
    }

    result
}

/// Resolve numeric label references in a single operand.
fn resolve_numeric_operand(
    op: &Operand,
    current_idx: usize,
    defs: &FxHashMap<String, Vec<(usize, String)>>,
) -> Operand {
    match op {
        Operand::Symbol(name) => {
            if let Some(resolved) = resolve_numeric_name(name, current_idx, defs) {
                Operand::Symbol(resolved)
            } else {
                op.clone()
            }
        }
        Operand::Label(name) => {
            if let Some(resolved) = resolve_numeric_name(name, current_idx, defs) {
                Operand::Label(resolved)
            } else {
                op.clone()
            }
        }
        Operand::SymbolOffset(name, off) => {
            if let Some(resolved) = resolve_numeric_name(name, current_idx, defs) {
                Operand::SymbolOffset(resolved, *off)
            } else {
                op.clone()
            }
        }
        Operand::MemExpr {
            base,
            expr,
            writeback,
        } => {
            let resolved_expr = resolve_numeric_refs_in_expr(expr, current_idx, defs);
            Operand::MemExpr {
                base: base.clone(),
                expr: resolved_expr,
                writeback: *writeback,
            }
        }
        Operand::Expr(expr) => {
            let resolved_expr = resolve_numeric_refs_in_expr(expr, current_idx, defs);
            Operand::Expr(resolved_expr)
        }
        _ => op.clone(),
    }
}

/// Resolve numeric label references (e.g., `1b`, `2f`) within an expression
/// string ([`crate::backend::asm_expr::rewrite_numeric_label_refs`]).
fn resolve_numeric_refs_in_expr(
    expr: &str,
    current_idx: usize,
    defs: &FxHashMap<String, Vec<(usize, String)>>,
) -> String {
    crate::backend::asm_expr::rewrite_numeric_label_refs(expr, |r| {
        resolve_numeric_name(r, current_idx, defs)
    })
}

/// Resolve numeric label references in the directives that carry
/// expressions: data values, `.org` (the kernel's ALTERNATIVE length checks,
/// `.org . - (664b-663b) + (662b-661b)`) and `.set`.
fn resolve_numeric_directive(
    dir: &AsmDirective,
    current_idx: usize,
    defs: &FxHashMap<String, Vec<(usize, String)>>,
) -> AsmDirective {
    let expr = |e: &str| resolve_numeric_refs_in_expr(e, current_idx, defs);
    let vals = |vals: &[DataValue]| -> Vec<DataValue> {
        vals.iter()
            .map(|v| match v {
                DataValue::Expr(e) => DataValue::Expr(expr(e)),
                DataValue::Integer(_) => v.clone(),
            })
            .collect()
    };
    match dir {
        AsmDirective::Byte(v) => AsmDirective::Byte(vals(v)),
        AsmDirective::Short(v) => AsmDirective::Short(vals(v)),
        AsmDirective::Long(v) => AsmDirective::Long(vals(v)),
        AsmDirective::Quad(v) => AsmDirective::Quad(vals(v)),
        // `.uleb128 2f-1f` (hand-written exception tables): the operands
        // name numeric labels like any other data value.
        AsmDirective::Leb128 { signed, values } => AsmDirective::Leb128 {
            signed: *signed,
            values: vals(values),
        },
        AsmDirective::Org { expr: e, fill } => AsmDirective::Org {
            expr: expr(e),
            fill: *fill,
        },
        AsmDirective::Set(name, e) => AsmDirective::Set(name.clone(), expr(e)),
        _ => dir.clone(),
    }
}

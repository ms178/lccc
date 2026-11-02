//! Saturating low-byte mask idiom selection. See the proof at `fuse_masks`.

use super::super::types::*;

use super::liveness::FileLiveness;

struct Op<'a> {
    mnemonic: &'a str,
    args: Vec<&'a str>,
}

fn parse(line: &str) -> Option<Op<'_>> {
    // No statements, comments, expressions, memory operands or aliases can
    // hide an extra operation in the strict register-only window below.
    if line.contains([';', '#', '\n']) {
        return None;
    }
    let (mnemonic, args) = line.trim().split_once(char::is_whitespace)?;
    let args: Vec<_> = args.split(',').map(str::trim).collect();
    if args.iter().any(|s| s.is_empty()) {
        return None;
    }
    Some(Op { mnemonic, args })
}

fn immediate(s: &str, expected: u64) -> bool {
    let Some(s) = s.strip_prefix('$') else {
        return false;
    };
    let value = if let Some(s) = s.strip_prefix("0x") {
        u64::from_str_radix(s, 16)
    } else {
        s.parse()
    };
    value == Ok(expected)
}

fn register(s: &str, size: MoveSize) -> Option<RegId> {
    let family = register_family_fast(s);
    // Stack/frame pointers are deliberately outside the admission set.
    (family <= REG_GP_MAX
        && family != REG_RSP
        && family != REG_RBP
        && s == reg_id_to_name(family, size))
    .then_some(family)
}

struct Mask {
    lines: Vec<usize>,
    size: MoveSize,
    index: RegId,
    source: RegId,
    mask: RegId,
    middle: RegId,
    dest: RegId,
}

fn match_mask(store: &LineStore, infos: &[LineInfo], start: usize) -> Option<Mask> {
    let first = parse(store.get(start))?;
    let (size, suffix, width) = match first.mnemonic {
        "andq" => (MoveSize::Q, 'q', 64),
        "andl" => (MoveSize::L, 'l', 32),
        _ => return None,
    };
    if first.args.len() != 2 || !immediate(first.args[0], 255) {
        return None;
    }
    let index = register(first.args[1], size)?;
    // Only deleted lines are transparent. Labels, directives (including CFI
    // and alignment), comments and real instructions break adjacency.
    let mut lines = vec![start];
    for _ in 1..8 {
        let next = next_non_nop(infos, lines.last()? + 1, infos.len());
        if next == infos.len() {
            return None;
        }
        lines.push(next);
    }
    let one = parse(store.get(lines[1]))?;
    if one.args.len() != 2 || !immediate(one.args[0], 1) {
        return None;
    }
    let mask = match one.mnemonic {
        "movl" => register(one.args[1], MoveSize::L)?,
        "movq" if size == MoveSize::Q => register(one.args[1], MoveSize::Q)?,
        _ => return None,
    };
    let shift = parse(store.get(lines[2]))?;
    if shift.mnemonic != format!("shlx{suffix}")
        || shift.args.len() != 3
        || register(shift.args[0], size)? != index
        || register(shift.args[1], size)? != mask
        || register(shift.args[2], size)? != mask
    {
        return None;
    }
    let sub = parse(store.get(lines[3]))?;
    if sub.mnemonic != format!("sub{suffix}")
        || sub.args.len() != 2
        || !immediate(sub.args[0], 1)
        || register(sub.args[1], size)? != mask
    {
        return None;
    }
    let copy = parse(store.get(lines[4]))?;
    if copy.mnemonic != format!("mov{suffix}") || copy.args.len() != 2 {
        return None;
    }
    let source = register(copy.args[0], size)?;
    let middle = register(copy.args[1], size)?;
    let families = [index, mask, source, middle];
    for (n, family) in families.iter().enumerate() {
        if families[..n].contains(family) {
            return None;
        }
    }
    let and = parse(store.get(lines[5]))?;
    if and.mnemonic != format!("and{suffix}")
        || and.args.len() != 2
        || register(and.args[0], size)? != mask
        || register(and.args[1], size)? != middle
    {
        return None;
    }
    let cmp = parse(store.get(lines[6]))?;
    if cmp.mnemonic != format!("cmp{suffix}")
        || cmp.args.len() != 2
        || !immediate(cmp.args[0], width)
        || register(cmp.args[1], size)? != index
    {
        return None;
    }
    let seventh = parse(store.get(lines[7]))?;
    let dest = if seventh.mnemonic == format!("mov{suffix}") {
        if seventh.args.len() != 2 || register(seventh.args[0], size)? != middle {
            return None;
        }
        let dest = register(seventh.args[1], size)?;
        let next = next_non_nop(infos, lines[7] + 1, infos.len());
        if next == infos.len() {
            return None;
        }
        lines.push(next);
        dest
    } else {
        middle
    };
    // The result is written before the retained CMP; neither CMP's operand
    // nor the original saturation value may be destroyed by that write.
    if dest == index || dest == source {
        return None;
    }
    let cmov = parse(store.get(*lines.last()?))?;
    if cmov.mnemonic != format!("cmovae{suffix}")
        || cmov.args.len() != 2
        || register(cmov.args[0], size)? != source
        || register(cmov.args[1], size)? != dest
        || !all_editable(infos, &lines)
    {
        return None;
    }
    Some(Mask {
        lines,
        size,
        index,
        source,
        mask,
        middle,
        dest,
    })
}

/// Select BZHI from an existing BMI2 sequence, not from a guessed target flag.
///
/// The retained AND proves index in 0..=255. For index < width, SHLX/SUB/AND
/// constructs exactly the low-bit mask; otherwise CMOVAE restores the source.
/// BZHI has precisely this low-byte/saturating contract, including index zero
/// and 256 wraparound BEFORE the operation. The original CMP is retained after
/// BZHI, preserving *all* arithmetic flags (including AF), not merely ZF/CF.
///
/// The window has no entry label, observable memory access, pinned instruction
/// or directive. All operand widths and aliases are checked. Discarded mask
/// and relay definitions must be dead on EVERY resolved CFG path; an unknown
/// exit refuses the rewrite. Output and index register states are unchanged.
/// The existing SHLX proves BMI2 was already required, so this never introduces
/// that ISA requirement into baseline code. Other backends/headers are untouched.
pub(super) fn fuse_masks(store: &mut LineStore, infos: &mut [LineInfo]) -> bool {
    let mut changed = false;
    let mut liveness = None;
    let mut i = 0;
    while i < infos.len() {
        if infos[i].is_nop() || infos[i].pinned {
            i += 1;
            continue;
        }
        let text = infos[i].trimmed(store.get(i));
        if !text.starts_with("andq") && !text.starts_with("andl") {
            i += 1;
            continue;
        }
        let Some(m) = match_mask(store, infos, i) else {
            i += 1;
            continue;
        };
        let last = *m.lines.last().expect("matched nonempty window");
        let lv = liveness.get_or_insert_with(|| FileLiveness::new(store, infos));
        if [m.mask, m.middle]
            .iter()
            .any(|&r| r != m.dest && lv.live_after(last, r) != Some(false))
        {
            i += 1;
            continue;
        }
        let suffix = if m.size == MoveSize::Q { 'q' } else { 'l' };
        let replacement = format!(
            "    bzhi{suffix} {}, {}, {}",
            reg_id_to_name(m.index, m.size),
            reg_id_to_name(m.source, m.size),
            reg_id_to_name(m.dest, m.size)
        );
        replace_line(store, &mut infos[m.lines[1]], m.lines[1], replacement);
        for (position, &line) in m.lines.iter().enumerate().skip(2) {
            if position != 6 {
                mark_nop(&mut infos[line]);
            }
        }
        lv.refresh_at(store, infos, i);
        changed = true;
        i = last + 1;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(width: char, relay: bool) -> String {
        let (index, mask, source, middle, one, bits) = if width == 'q' {
            ("%r8", "%r11", "%rdi", "%rax", "%r11d", 64)
        } else {
            ("%r8d", "%r11d", "%edi", "%eax", "%r11d", 32)
        };
        let mut s = format!(
            "and{width} $255, {index}\nmovl $1, {one}\nshlx{width} {index}, {mask}, {mask}\nsub{width} $1, {mask}\nmov{width} {source}, {middle}\nand{width} {mask}, {middle}\ncmp{width} ${bits}, {index}\n"
        );
        let dest = if relay { mask } else { middle };
        if relay {
            s += &format!("mov{width} {middle}, {dest}\n");
        }
        s += &format!("cmovae{width} {source}, {dest}\n");
        s
    }

    fn wrap(s: &str) -> String {
        format!("f:\n.cfi_startproc\n{s}ret\n.cfi_endproc\n")
    }

    fn run(asm: &str, pin: Option<usize>) -> (bool, String) {
        let mut store = LineStore::new(asm.to_owned());
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        super::super::pin_inline_asm_regions(&store, &mut infos);
        super::super::pin_volatile_access_regions(&store, &mut infos);
        if let Some(i) = pin {
            infos[i].pinned = true;
        }
        let changed = fuse_masks(&mut store, &mut infos);
        (changed, store.build_result(|i| infos[i].is_nop()))
    }

    #[test]
    fn selects_both_widths_and_keeps_exact_comparison() {
        for w in ['q', 'l'] {
            let asm = wrap(&body(w, false));
            let (changed, out) = run(&asm, None);
            assert!(changed, "{asm}");
            assert!(out.contains(&format!("bzhi{w} ")));
            assert!(!out.contains("shlx"));
            let cmp = asm.lines().find(|s| s.starts_with("cmp")).unwrap();
            assert!(out.lines().any(|s| s == cmp));
            assert!(!run(&out, None).0, "must be idempotent");
        }
    }

    #[test]
    fn selects_relay_into_mask_register_when_middle_is_dead() {
        for w in ['q', 'l'] {
            let mut s = body(w, true);
            // The relay's result is in r11; return it, killing the old middle.
            s += if w == 'q' {
                "movq %r11, %rax\n"
            } else {
                "movl %r11d, %eax\n"
            };
            assert!(run(&wrap(&s), None).0);
        }
    }

    #[test]
    fn retains_live_mask_or_relay_temporary_on_every_path() {
        for tail in [
            "movq %r11, %rdx\n",
            "testq %rsi, %rsi\njnz .Luse\njmp .Ldone\n.Luse:\nmovq %r11, %rdx\n.Ldone:\n",
            "jmp *%rsi\n",
        ] {
            assert!(!run(&wrap(&(body('q', false) + tail)), None).0);
        }
        assert!(
            !run(
                &wrap(&(body('q', true) + "movq %rax, %rdx\nmovq %r11, %rax\n")),
                None
            )
            .0
        );
        assert!(!run(&body('q', false), None).0, "unknown function exit");
    }

    #[test]
    fn refuses_width_alias_condition_and_saturation_near_misses() {
        let s = body('q', false);
        for (from, to) in [
            ("$255", "$511"),
            ("$255", "$254"),
            ("$64", "$63"),
            ("cmovaeq", "cmovbq"),
            ("cmovaeq", "cmovgeq"),
            ("shlxq", "shrxq"),
            ("subq $1", "subq $2"),
            ("movl $1", "movl $2"),
            ("andq %r11", "andl %r11d"),
            ("%rdi", "%r8"),
            ("%rax", "%r8"),
            ("%rdi", "%r11"),
            ("%rax", "%r11"),
            ("%rdi", "%rax"),
            ("%r8", "%rsp"),
        ] {
            let test = wrap(&s.replace(from, to));
            assert!(!run(&test, None).0, "must refuse {from} -> {to}: {test}");
        }
    }

    #[test]
    fn respects_every_pin_opaque_region_and_interior_barrier() {
        let s = body('q', false);
        for pin in 2..10 {
            assert!(!run(&wrap(&s), Some(pin)).0, "pin {pin}");
        }
        assert!(!run(&wrap(&format!("#APP\n{s}#NO_APP\n")), None).0);
        for barrier in [
            ".byte 0x90",
            ".p2align 4",
            ".Lentry:",
            ".rept 2",
            ".cfi_remember_state",
            "nop",
            "# unknown marker",
        ] {
            for at in 1..8 {
                let mut lines: Vec<_> = s.lines().collect();
                lines.insert(at, barrier);
                assert!(
                    !run(&wrap(&(lines.join("\n") + "\n")), None).0,
                    "{barrier} at {at}"
                );
            }
        }
    }

    #[test]
    fn accepts_whitespace_and_hex_mask_without_relaxing_operands() {
        let s = body('q', false).replace("$255", "$0xff");
        let s = s
            .lines()
            .map(|s| format!("\t{s}  \t\n"))
            .collect::<String>();
        assert!(run(&wrap(&s), None).0);
        assert!(
            !run(
                &wrap(&body('q', false).replace("%rdi, %rax", "(%rdi), %rax")),
                None
            )
            .0
        );
        assert!(
            !run(
                &wrap(&body('q', false).replace("movl $1, %r11d", "movl $1, %r11d; incq %rsi")),
                None
            )
            .0
        );
    }
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    #[test]
    fn native_results_and_all_arithmetic_flags_match_independent_model() {
        if !std::is_x86_feature_detected!("bmi2") {
            eprintln!("native BMI2 mask check unavailable: host lacks BMI2");
            return;
        }
        use crate::common::temp_files::TempFile;
        use std::process::Command;
        let asm = TempFile::try_new("bmi2-mask", "native", "s").unwrap();
        let c = TempFile::try_new("bmi2-mask", "native", "c").unwrap();
        let exe = TempFile::try_new("bmi2-mask", "native", "exe").unwrap();
        let mut assembly = String::from(".text\n");
        for (number, (width, relay)) in [('q', false), ('l', false), ('q', true), ('l', true)]
            .iter()
            .enumerate()
        {
            let mut code = body(*width, *relay);
            if *relay {
                code += if *width == 'q' {
                    "movq %r11, %rax\n"
                } else {
                    "movl %r11d, %eax\n"
                };
            }
            // Preserve RAX and capture flags before any flag-writing check.
            code += "pushfq\npopq %r10\nmovq %r10, (%rdx)\n";
            let original = wrap(&("movq %rsi, %r8\n".to_owned() + &code));
            let (changed, selected) = run(&original, None);
            assert!(changed);
            let pipeline = super::super::peephole_optimize(original.clone());
            assert!(
                pipeline.contains("bzhi"),
                "full pipeline must select the idiom: {pipeline}"
            );
            for (stage, code) in [(0, original), (1, selected), (2, pipeline)] {
                let name = format!("f{number}_{stage}");
                assembly += &format!(
                    ".globl {name}\n.type {name}, @function\n{}",
                    code.replacen("f:", &format!("{name}:"), 1)
                );
            }
        }
        assembly += ".section .note.GNU-stack,\"\",@progbits\n";
        std::fs::write(asm.path(), assembly).unwrap();
        let declarations = (0..4)
            .flat_map(|n| {
                (0..3).map(move |stage| {
                    format!("extern uint64_t f{n}_{stage}(uint64_t,uint64_t,uint64_t*);\n")
                })
            })
            .collect::<String>();
        let entries = (0..4)
            .flat_map(|n| (0..3).map(move |stage| format!("f{n}_{stage},")))
            .collect::<String>();
        let source = format!(
            r#"
#include <stdint.h>
{declarations}
int main(void) {{
 uint64_t (*f[])(uint64_t,uint64_t,uint64_t*) = {{{entries}}};
 uint64_t state=0x123456789abcdefULL;
 for (unsigned k=0;k<65536;k++) {{
  state=state*6364136223846793005ULL+1;
  uint64_t index=(state&~65535ULL)|k, n=index&255;
  uint64_t value=(k&1)?state:~0ULL;
  for (unsigned j=0;j<12;j++) {{
   unsigned width=((j/3)&1)?32:64;
   uint64_t mask=width==32?0xffffffffULL:~0ULL;
   uint64_t x=value&mask;
   uint64_t expected=n>=width?x:x&((1ULL<<n)-1);
   uint64_t diff=(n-width)&mask, flags=0;
   unsigned pf=1; for (unsigned b=0;b<8;b++) pf^=(diff>>b)&1;
   uint64_t ef=(n<width)|((uint64_t)pf<<2)|(((n^width^diff)&16))|
       ((uint64_t)(diff==0)<<6)|(((diff>>(width-1))&1)<<7)|
       ((((n^width)&(n^diff))>>(width-1)&1)<<11);
   if (f[j](value,index,&flags)!=expected || (flags&0x8d5)!=ef) return j+1;
  }}
 }}
 return 0;
}}
"#
        );
        std::fs::write(c.path(), source).unwrap();
        let compiled = Command::new("gcc")
            .args(["-O2", "-Werror"])
            .arg(c.path())
            .arg(asm.path())
            .arg("-o")
            .arg(exe.path())
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let result = Command::new(exe.path()).output().unwrap();
        assert!(
            result.status.success(),
            "native result/flags mismatch: {:?}",
            result.status
        );
    }
}

//! Every shape the multi-word folds accept is executed before and after on
//! a small register, memory and carry-flag model, over edge and
//! pseudo-random states; every shape they must refuse is pinned as unchanged.

use super::*;
use crate::common::fx_hash::FxHashMap;

fn run_folds(body: &str) -> (bool, String) {
    let text = format!("foo:\n.cfi_startproc\n{body}\n.cfi_endproc\n");
    let mut store = LineStore::new(text);
    let mut infos: Vec<_> = (0..store.len())
        .map(|i| classify_line(store.get(i)))
        .collect();
    let mut changed = false;
    loop {
        let a = fold_carry_compare(&mut store, &mut infos);
        let b = fold_carry_into_adc(&mut store, &mut infos);
        let c = fold_carry_addend(&mut store, &mut infos);
        let d = fold_product_pair(&mut store, &mut infos);
        if !(a || b || c || d) {
            break;
        }
        changed = true;
    }
    let out = (0..store.len())
        .filter(|&i| !infos[i].is_nop())
        .map(|i| store.get(i).trim())
        .filter(|line| !line.starts_with(".cfi") && *line != "foo:")
        .collect::<Vec<_>>()
        .join("\n");
    (changed, out)
}

/// Sixteen registers, 8-byte memory cells keyed by address, and CF.
#[derive(Clone, PartialEq, Debug)]
struct Cpu {
    regs: [u64; 16],
    memory: FxHashMap<u64, u64>,
    carry: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Size {
    B,
    L,
    Q,
}

enum Place {
    Reg(usize),
    Mem(u64),
    Imm(u64),
}

impl Cpu {
    fn place(&self, text: &str) -> Place {
        let text = text.trim();
        if let Some(value) = text.strip_prefix('$') {
            return Place::Imm(value.parse::<i64>().expect("immediate") as u64);
        }
        if text.starts_with('%') {
            for tier in [0, 1, 3] {
                if let Some(fam) = REG_NAMES[tier].iter().position(|&name| name == text) {
                    return Place::Reg(fam);
                }
            }
            panic!("model has no register {text}");
        }
        Place::Mem(self.address(text))
    }

    /// `disp(%base)`, `(%base,%index)`, `(%base,%index,scale)`.
    fn address(&self, text: &str) -> u64 {
        let open = text.find('(').expect("memory operand");
        let disp = match text[..open].trim() {
            "" => 0i64,
            literal => literal.parse().expect("displacement"),
        };
        let inner = text[open + 1..].strip_suffix(')').expect("closing paren");
        let mut parts = inner.split(',').map(str::trim);
        let reg = |name: &str| {
            let fam = REG_NAMES[0]
                .iter()
                .position(|&n| n == name)
                .unwrap_or_else(|| panic!("address register {name}"));
            self.regs[fam]
        };
        let mut address = (disp as u64).wrapping_add(reg(parts.next().expect("base")));
        if let Some(index) = parts.next() {
            let scale: u64 = parts.next().map_or(1, |s| s.parse().expect("scale"));
            address = address.wrapping_add(reg(index).wrapping_mul(scale));
        }
        address
    }

    fn read(&self, place: &Place, size: Size) -> u64 {
        let value = match place {
            Place::Reg(fam) => self.regs[*fam],
            Place::Mem(address) => *self.memory.get(address).unwrap_or(&0x5a5a_5a5a_a5a5_a5a5),
            Place::Imm(value) => *value,
        };
        match size {
            Size::B => value & 0xff,
            Size::L => value & 0xffff_ffff,
            Size::Q => value,
        }
    }

    fn write(&mut self, place: &Place, size: Size, value: u64) {
        match place {
            Place::Reg(fam) => {
                self.regs[*fam] = match size {
                    Size::B => (self.regs[*fam] & !0xff) | (value & 0xff),
                    Size::L => value & 0xffff_ffff,
                    Size::Q => value,
                }
            }
            Place::Mem(address) => {
                assert!(size == Size::Q, "model stores whole cells only");
                self.memory.insert(*address, value);
            }
            Place::Imm(_) => panic!("write to an immediate"),
        }
    }

    fn step(&mut self, line: &str) {
        let (mnem, args) = line.split_once(' ').unwrap_or((line, ""));
        let operands: Vec<&str> = match top_level_operands(args) {
            Some((a, b)) => vec![a, b],
            None => vec![args.trim()],
        };
        let size = match mnem.chars().last() {
            Some('q') => Size::Q,
            Some('l') => Size::L,
            _ => Size::B,
        };
        let limit = if size == Size::Q {
            u128::from(u64::MAX)
        } else {
            0xffff_ffff
        };
        match mnem {
            "movq" | "movl" => {
                let value = self.read(&self.place(operands[0]), size);
                let dst = self.place(operands[1]);
                self.write(&dst, size, value);
            }
            "movzbl" | "movzbq" => {
                let value = self.read(&self.place(operands[0]), Size::B);
                let dst = self.place(operands[1]);
                self.write(&dst, Size::Q, value);
            }
            "leaq" | "leal" => {
                let value = self.address(operands[0]);
                let dst = self.place(operands[1]);
                self.write(&dst, size, value);
            }
            "addq" | "addl" | "adcq" | "adcl" => {
                let dst = self.place(operands[1]);
                let carry_in = u128::from(mnem.starts_with("adc") && self.carry);
                let sum = u128::from(self.read(&dst, size))
                    + u128::from(self.read(&self.place(operands[0]), size))
                    + carry_in;
                self.carry = sum > limit;
                self.write(&dst, size, sum as u64);
            }
            "subq" | "subl" | "sbbq" | "sbbl" | "cmpq" | "cmpl" => {
                let dst = self.place(operands[1]);
                let borrow_in = u128::from(mnem.starts_with("sbb") && self.carry);
                let minuend = u128::from(self.read(&dst, size));
                let subtrahend = u128::from(self.read(&self.place(operands[0]), size)) + borrow_in;
                self.carry = minuend < subtrahend;
                if !mnem.starts_with("cmp") {
                    self.write(&dst, size, minuend.wrapping_sub(subtrahend) as u64);
                }
            }
            // Products: only the register results are modelled; the flags
            // a multiply leaves are never read by the shapes under test.
            "mulq" | "imulq" if operands.len() == 1 => {
                let factor = self.read(&self.place(operands[0]), Size::Q);
                let product = if mnem == "mulq" {
                    u128::from(self.regs[0]) * u128::from(factor)
                } else {
                    (i128::from(self.regs[0] as i64) * i128::from(factor as i64)) as u128
                };
                self.regs[0] = product as u64;
                self.regs[2] = (product >> 64) as u64;
                self.carry = false;
            }
            "imulq" => {
                let dst = self.place(operands[1]);
                let product = self
                    .read(&dst, Size::Q)
                    .wrapping_mul(self.read(&self.place(operands[0]), Size::Q));
                self.write(&dst, Size::Q, product);
                self.carry = false;
            }
            "setb" | "setc" | "setnae" => {
                let dst = self.place(operands[0]);
                self.write(&dst, Size::B, u64::from(self.carry));
            }
            "setae" | "setnb" | "setnc" => {
                let dst = self.place(operands[0]);
                self.write(&dst, Size::B, u64::from(!self.carry));
            }
            "ret" => {}
            other => panic!("model has no `{other}` ({line})"),
        }
    }

    fn run(mut self, body: &str) -> Self {
        for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
            self.step(line);
        }
        self
    }
}

fn top_level_operands(args: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    let mut at = None;
    for (i, c) in args.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => at = Some(i),
            _ => {}
        }
    }
    at.map(|i| (args[..i].trim(), args[i + 1..].trim()))
}

fn states() -> Vec<Cpu> {
    const EDGES: [u64; 8] = [
        0,
        1,
        0xffff_ffff,
        0x1_0000_0000,
        0x7fff_ffff_ffff_ffff,
        0x8000_0000_0000_0000,
        0xffff_ffff_ffff_fffe,
        0xffff_ffff_ffff_ffff,
    ];
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut all = Vec::new();
    for round in 0..400usize {
        let mut regs = [0u64; 16];
        for (i, reg) in regs.iter_mut().enumerate() {
            // Mix edges in so that carries and borrows actually happen.
            *reg = if (round + i) % 3 == 0 {
                EDGES[(next() % 8) as usize]
            } else {
                next()
            };
        }
        // Stack pointer and one data pointer: aligned, distinct, stable.
        regs[4] = 0x7000_0000;
        regs[7] = 0x6000_0000;
        let mut memory = FxHashMap::default();
        for cell in 0..8u64 {
            memory.insert(
                0x7000_0000 + cell * 8,
                EDGES[(next() % 8) as usize] ^ (next() & 1),
            );
            memory.insert(0x6000_0000 + cell * 8, next());
        }
        all.push(Cpu {
            regs,
            memory,
            carry: round % 2 == 0,
        });
    }
    all
}

/// Fold `body` and require the result to compute the same registers (all
/// but the `scratch` families, which the body declares dead) and memory.
fn folded(body: &str, scratch: &[&str]) -> String {
    let (changed, out) = run_folds(body);
    assert!(changed, "not folded:\n{body}");
    let scratch: Vec<usize> = scratch
        .iter()
        .map(|name| {
            REG_NAMES[0]
                .iter()
                .position(|n| n == name)
                .expect("scratch register")
        })
        .collect();
    for state in states() {
        let before = state.clone().run(body);
        let after = state.clone().run(&out);
        for fam in (0..16).filter(|fam| !scratch.contains(fam)) {
            assert_eq!(
                before.regs[fam], after.regs[fam],
                "{} differs\n--- before\n{body}\n--- after\n{out}\n{state:x?}",
                REG_NAMES[0][fam]
            );
        }
        assert_eq!(before.memory, after.memory, "memory\n{body}\n=>\n{out}");
    }
    out
}

fn unchanged(body: &str) {
    let (changed, out) = run_folds(body);
    assert!(!changed, "must not fold:\n{body}\n=>\n{out}");
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().map(str::trim).collect()
}

/// `setb; movzbl` into %r10 after whatever set the flags.
const CARRY: &str = "    setb %r10b\n    movzbl %r10b, %r10d";

#[test]
fn the_model_computes_carries_and_borrows() {
    let mut cpu = states().remove(0);
    cpu.regs[0] = u64::MAX;
    cpu.regs[1] = 1;
    cpu.regs[3] = 5;
    let cpu = cpu.run("addq %rcx, %rax\nsetb %dl\nmovzbl %dl, %edx\nadcq $0, %rbx");
    assert_eq!((cpu.regs[0], cpu.regs[2], cpu.regs[3]), (0, 1, 6));
    let mut cpu = states().remove(1);
    cpu.regs[0] = 0;
    cpu.regs[1] = 1;
    cpu.regs[3] = 10;
    let cpu = cpu.run("cmpq %rcx, %rax\nsbbq $0, %rbx\nsubl %ecx, %eax\nsetae %dl");
    assert_eq!(cpu.regs[3], 9);
    assert_eq!(cpu.regs[0], 0xffff_ffff);
    assert_eq!(cpu.regs[2] & 0xff, 0);
    let mut cpu = states().remove(2);
    cpu.regs[0] = u64::MAX;
    cpu.regs[1] = u64::MAX;
    let cpu = cpu.run("mulq %rcx");
    assert_eq!((cpu.regs[0], cpu.regs[2]), (1, u64::MAX - 1));
}

#[test]
fn an_in_place_add_and_its_carry_become_add_adc() {
    // The bignum step the 128-bit split emits.
    let out = folded(
        &format!("    addq %rdx, %rsi\n    cmpq %rdx, %rsi\n{CARRY}\n    addq %r10, %rbx\n    ret"),
        &["%r10"],
    );
    assert_eq!(lines(&out), ["addq %rdx, %rsi", "adcq $0, %rbx", "ret"]);

    for (add, cmp) in [
        ("addq 8(%rsp), %rsi", "cmpq 8(%rsp), %rsi"),
        ("addq $77, %rsi", "cmpq $77, %rsi"),
        ("addq $-3, %rsi", "cmpq $-3, %rsi"),
        ("addq (%rdi), %rsi", "cmpq (%rdi), %rsi"),
        ("addl %edx, %esi", "cmpl %edx, %esi"),
        ("addl 16(%rsp), %esi", "cmpl 16(%rsp), %esi"),
    ] {
        let wide = if add.starts_with("addq") { 'q' } else { 'l' };
        let (carry, acc) = if wide == 'q' {
            ("%r10", "%rbx")
        } else {
            ("%r10d", "%ebx")
        };
        let out = folded(
            &format!("    {add}\n    {cmp}\n{CARRY}\n    add{wide} {carry}, {acc}\n    ret"),
            &["%r10"],
        );
        assert_eq!(
            lines(&out),
            [add, &format!("adc{wide} $0, {acc}"), "ret"],
            "{add}"
        );
    }
}

#[test]
fn a_copied_addend_may_be_the_one_compared() {
    let out = folded(
        "    movq %r11, %rax\n    addq 16(%rsp), %rax\n    cmpq %r11, %rax\n    setc %r10b\n    movzbq %r10b, %r10\n    addq %r10, %rbx\n    ret",
        &["%r10"],
    );
    assert_eq!(
        lines(&out),
        [
            "movq %r11, %rax",
            "addq 16(%rsp), %rax",
            "adcq $0, %rbx",
            "ret"
        ]
    );
}

#[test]
fn an_lea_sum_is_rebuilt_as_the_flag_setting_add() {
    for cmp in ["cmpq %rdx, %r15", "cmpq %rax, %r15"] {
        let out = folded(
            &format!(
                "    leaq (%rax, %rdx), %r15\n    {cmp}\n{CARRY}\n    addq %r10, %rbx\n    ret"
            ),
            &["%r10"],
        );
        assert_eq!(
            lines(&out),
            ["movq %rax, %r15", "addq %rdx, %r15", "adcq $0, %rbx", "ret"]
        );
    }
    let out = folded(
        &format!(
            "    leaq (%rsi,%r9,1), %r11\n    cmpq %rsi, %r11\n{CARRY}\n    addq %r10, %rbx\n    ret"
        ),
        &["%r10"],
    );
    assert_eq!(lines(&out)[..2], ["movq %rsi, %r11", "addq %r9, %r11"]);

    for (lea, cmp, add) in [
        ("leaq 1(%rax), %rcx", "cmpq $1, %rcx", "addq $1, %rcx"),
        ("leaq -8(%rax), %rcx", "cmpq $-8, %rcx", "addq $-8, %rcx"),
        ("leaq 40(%rax), %rcx", "cmpq %rax, %rcx", "addq $40, %rcx"),
    ] {
        let out = folded(
            &format!("    {lea}\n    {cmp}\n{CARRY}\n    addq %r10, %rbx\n    ret"),
            &["%r10"],
        );
        assert_eq!(
            lines(&out),
            ["movq %rax, %rcx", add, "adcq $0, %rbx", "ret"]
        );
    }
    let out = folded(
        &format!(
            "    leal (%rax, %rdx), %r15d\n    cmpl %edx, %r15d\n{CARRY}\n    addl %r10d, %ebx\n    ret"
        ),
        &["%r10"],
    );
    assert_eq!(
        lines(&out),
        [
            "movl %eax, %r15d",
            "addl %edx, %r15d",
            "adcl $0, %ebx",
            "ret"
        ]
    );
}

#[test]
fn a_subtraction_and_its_borrow_become_sub_sbb() {
    let out = folded(
        &format!(
            "    movq %r9, %rax\n    subq %rdx, %rax\n    cmpq %rdx, %r9\n{CARRY}\n    subq %r10, %rsi\n    ret"
        ),
        &["%r10"],
    );
    assert_eq!(
        lines(&out),
        ["movq %r9, %rax", "subq %rdx, %rax", "sbbq $0, %rsi", "ret"]
    );
    let out = folded(
        &format!(
            "    movl %r9d, %eax\n    subl 8(%rsp), %eax\n    cmpl 8(%rsp), %r9d\n{CARRY}\n    subl %r10d, %esi\n    ret"
        ),
        &["%r10"],
    );
    assert_eq!(lines(&out)[2], "sbbl $0, %esi");
}

#[test]
fn every_consumer_form_matches_its_identity() {
    // The flags come from a compare this pass does not touch.
    let producer = "    cmpq %r8, %r9";
    for (set, tail, expected) in [
        ("setb", "addq %r10, %rbx", vec!["adcq $0, %rbx"]),
        ("setb", "subq %r10, %rbx", vec!["sbbq $0, %rbx"]),
        ("setae", "addq %r10, %rbx", vec!["sbbq $-1, %rbx"]),
        ("setae", "subq %r10, %rbx", vec!["adcq $-1, %rbx"]),
        ("setnc", "addl %r10d, %ebx", vec!["sbbl $-1, %ebx"]),
        ("setnae", "subl %r10d, %ebx", vec!["sbbl $0, %ebx"]),
        // carry + value, into the carry register.
        (
            "setb",
            "addq %rbx, %r10",
            vec!["movq %rbx, %r10", "adcq $0, %r10"],
        ),
        (
            "setb",
            "addq 32(%rsp), %r10",
            vec!["movq 32(%rsp), %r10", "adcq $0, %r10"],
        ),
        (
            "setae",
            "addl 32(%rsp), %r10d",
            vec!["movl 32(%rsp), %r10d", "sbbl $-1, %r10d"],
        ),
        // The three-operand add.
        (
            "setb",
            "leaq (%r10, %rbx), %rcx",
            vec!["movq %rbx, %rcx", "adcq $0, %rcx"],
        ),
        (
            "setb",
            "leaq (%rbx,%r10,1), %rcx",
            vec!["movq %rbx, %rcx", "adcq $0, %rcx"],
        ),
        ("setb", "leaq (%r10, %rbx), %rbx", vec!["adcq $0, %rbx"]),
        (
            "setb",
            "leaq (%r10, %rbx), %r10",
            vec!["movq %rbx, %r10", "adcq $0, %r10"],
        ),
        (
            "setae",
            "leaq (%r10, %rbx), %r8",
            vec!["movq %rbx, %r8", "sbbq $-1, %r8"],
        ),
    ] {
        let lands_in_carry = tail.ends_with("%r10") || tail.ends_with("%r10d");
        let scratch: &[&str] = if lands_in_carry { &[] } else { &["%r10"] };
        let body =
            format!("{producer}\n    {set} %r10b\n    movzbl %r10b, %r10d\n    {tail}\n    ret");
        let out = folded(&body, scratch);
        let mut want = vec!["cmpq %r8, %r9"];
        want.extend(expected.iter().copied());
        want.push("ret");
        assert_eq!(lines(&out), want, "{set} / {tail}");
    }
}

#[test]
fn a_load_staged_between_the_carry_and_its_add_stays_in_place() {
    for (load, tail, update) in [
        ("movq 16(%rsp), %r15", "addq %r10, %r15", "adcq $0, %r15"),
        ("movq %rbx, %r15", "addq %r10, %r15", "adcq $0, %r15"),
        ("movq $7, %r15", "addq %r10, %r15", "adcq $0, %r15"),
        ("movq (%rdi), %r15", "subq %r10, %r15", "sbbq $0, %r15"),
        (
            "movl 16(%rsp), %r15d",
            "addl %r10d, %r15d",
            "adcl $0, %r15d",
        ),
    ] {
        let out = folded(
            &format!("    cmpq %r8, %r9\n{CARRY}\n    {load}\n    {tail}\n    ret"),
            &["%r10"],
        );
        assert_eq!(lines(&out), ["cmpq %r8, %r9", load, update, "ret"]);
    }
    // The staged load must not involve the carry, and must load the register
    // the carry is then added to.
    for (load, tail) in [
        ("movq %r10, %r15", "addq %r10, %r15"),
        ("movq (%r10), %r15", "addq %r10, %r15"),
        ("movq 16(%rsp), %r14", "addq %r10, %r15"),
        ("movq %r15, 16(%rsp)", "addq %r10, %r15"),
        ("addq $1, %r15", "addq %r10, %r15"),
    ] {
        unchanged(&format!(
            "    cmpq %r8, %r9\n{CARRY}\n    {load}\n    {tail}\n    ret"
        ));
    }
}

#[test]
fn an_addend_after_the_carry_in_joins_it() {
    for (first, second, merged) in [
        ("adcq $0, %rbx", "addq %rcx, %rbx", "adcq %rcx, %rbx"),
        ("adcq $0, %rbx", "addq 8(%rsp), %rbx", "adcq 8(%rsp), %rbx"),
        ("adcq $0, %rbx", "addq $-5, %rbx", "adcq $-5, %rbx"),
        ("adcl $0, %ebx", "addl %ecx, %ebx", "adcl %ecx, %ebx"),
        ("sbbq $0, %rbx", "subq %rcx, %rbx", "sbbq %rcx, %rbx"),
        ("sbbl $0, %ebx", "subl (%rdi), %ebx", "sbbl (%rdi), %ebx"),
    ] {
        let out = folded(
            &format!("    cmpq %r8, %r9\n    {first}\n    {second}\n    ret"),
            &[],
        );
        assert_eq!(lines(&out), ["cmpq %r8, %r9", merged, "ret"]);
    }
    let producer = "    cmpq %r8, %r9";
    // Mixed operations, another register, a self-referencing addend, a
    // different carry-in constant, and live flags all stay.
    unchanged(&format!(
        "{producer}\n    adcq $0, %rbx\n    subq %rcx, %rbx\n    ret"
    ));
    unchanged(&format!(
        "{producer}\n    sbbq $0, %rbx\n    addq %rcx, %rbx\n    ret"
    ));
    unchanged(&format!(
        "{producer}\n    adcq $0, %rbx\n    addq %rcx, %rdx\n    ret"
    ));
    unchanged(&format!(
        "{producer}\n    adcq $0, %rbx\n    addq %rbx, %rbx\n    ret"
    ));
    unchanged(&format!(
        "{producer}\n    adcq $0, %rbx\n    addq (%rbx), %rbx\n    ret"
    ));
    unchanged(&format!(
        "{producer}\n    adcq $1, %rbx\n    addq %rcx, %rbx\n    ret"
    ));
    unchanged(&format!(
        "{producer}\n    adcq $0, %rbx\n    addl %ecx, %ebx\n    ret"
    ));
    unchanged(&format!(
        "{producer}\n    adcq $0, %rbx\n    addq %rcx, %rbx\n    jb .Lx\n    ret\n.Lx:\n    ret"
    ));
}

/// The 128-bit add the split emits, with both high words live: low add,
/// carry test, the carry joining one high word, then the other high word.
#[test]
fn a_full_two_word_add_becomes_add_adc() {
    let out = folded(
        &format!(
            "    addq %rdx, %rsi\n    cmpq %rdx, %rsi\n{CARRY}\n    movq %r8, %r11\n    addq %r10, %r11\n    addq %rcx, %r11\n    ret"
        ),
        &["%r10"],
    );
    assert_eq!(
        lines(&out),
        [
            "addq %rdx, %rsi",
            "movq %r8, %r11",
            "adcq %rcx, %r11",
            "ret"
        ]
    );
    // In place, and the subtraction twin.
    let out = folded(
        &format!(
            "    addq %rdx, %rsi\n    cmpq %rdx, %rsi\n{CARRY}\n    addq %r10, %r8\n    addq 24(%rsp), %r8\n    ret"
        ),
        &["%r10"],
    );
    assert_eq!(
        lines(&out),
        ["addq %rdx, %rsi", "adcq 24(%rsp), %r8", "ret"]
    );
    let out = folded(
        &format!(
            "    movq %r9, %rax\n    subq %rdx, %rax\n    cmpq %rdx, %r9\n{CARRY}\n    subq %r10, %r8\n    subq %rcx, %r8\n    ret"
        ),
        &["%r10"],
    );
    assert_eq!(
        lines(&out),
        ["movq %r9, %rax", "subq %rdx, %rax", "sbbq %rcx, %r8", "ret"]
    );
}

#[test]
fn the_byte_register_may_differ_from_the_widened_one() {
    let out = folded(
        "    addq %rdx, %rsi\n    cmpq %rdx, %rsi\n    setb %r11b\n    movzbl %r11b, %r10d\n    addq %r10, %rbx\n    ret",
        &["%r10", "%r11"],
    );
    assert_eq!(lines(&out), ["addq %rdx, %rsi", "adcq $0, %rbx", "ret"]);
}

#[test]
fn two_chained_carries_fold_independently() {
    // t = p + o + carry_in; carry_out = hi + c1 + c2 -- the bignum step.
    let out = folded(
        "    addq %rdx, %rsi\n    cmpq %rdx, %rsi\n    setb %r10b\n    movzbl %r10b, %r10d\n    addq %r10, %rbx\n    addq %r8, %rsi\n    cmpq %r8, %rsi\n    setb %r11b\n    movzbl %r11b, %r10d\n    leaq (%r10, %rbx), %r8\n    ret",
        &["%r10", "%r11"],
    );
    assert_eq!(
        lines(&out),
        [
            "addq %rdx, %rsi",
            "adcq $0, %rbx",
            "addq %r8, %rsi",
            "movq %rbx, %r8",
            "adcq $0, %r8",
            "ret"
        ]
    );
}

#[test]
fn live_flags_or_a_live_boolean_refuse_the_consumer_fold() {
    let producer = "    cmpq %r8, %r9";
    // The add's flags are read afterwards.
    unchanged(&format!(
        "{producer}\n{CARRY}\n    addq %r10, %rbx\n    jne .Lx\n    ret\n.Lx:\n    ret"
    ));
    // The `lea` form leaves the compare's flags live; here they are read.
    unchanged(&format!(
        "{producer}\n{CARRY}\n    leaq (%r10, %rbx), %rcx\n    jb .Lx\n    ret\n.Lx:\n    ret"
    ));
    // The widened carry is read again.
    unchanged(&format!(
        "{producer}\n{CARRY}\n    addq %r10, %rbx\n    movq %r10, %rax\n    ret"
    ));
    // Not a carry condition.
    for set in ["setbe", "seta", "sete", "setl"] {
        unchanged(&format!(
            "{producer}\n    {set} %r10b\n    movzbl %r10b, %r10d\n    addq %r10, %rbx\n    ret"
        ));
    }
    // The extension reads a different byte.
    unchanged(&format!(
        "{producer}\n    setb %r10b\n    movzbl %r11b, %r10d\n    addq %r10, %rbx\n    ret"
    ));
    // A sign extension is not the 0/1 value.
    unchanged(&format!(
        "{producer}\n    setb %r10b\n    movsbl %r10b, %r10d\n    addq %r10, %rbx\n    ret"
    ));
    // `carry - value` has no single-instruction form.
    unchanged(&format!(
        "{producer}\n{CARRY}\n    subq %rbx, %r10\n    ret"
    ));
    // The stack and frame pointers are never rewritten.
    unchanged(&format!(
        "{producer}\n{CARRY}\n    addq %r10, %rsp\n    ret"
    ));
    // A scaled index, or a displacement, is not the carry itself.
    unchanged(&format!(
        "{producer}\n{CARRY}\n    leaq (%rbx,%r10,2), %rcx\n    ret"
    ));
    unchanged(&format!(
        "{producer}\n{CARRY}\n    leaq 8(%rbx,%r10), %rcx\n    ret"
    ));
    // The value operand is the byte register's own family.
    unchanged(&format!(
        "{producer}\n    setb %r11b\n    movzbl %r11b, %r10d\n    leaq (%r10, %r11), %rcx\n    ret"
    ));
    // The preloaded operand is addressed through the carry register.
    unchanged(&format!(
        "{producer}\n{CARRY}\n    addq (%r10), %r10\n    ret"
    ));
}

#[test]
fn a_compare_that_is_not_the_carry_or_is_read_for_more_stays() {
    let tail = "    setb %r10b\n    movzbl %r10b, %r10d\n    movq %r10, (%rdi)\n    ret";
    // A different comparand: not the carry of the add.
    unchanged(&format!("    addq %rdx, %rsi\n    cmpq %rcx, %rsi\n{tail}"));
    // The sum compared from the other side (`x <u s`): not the carry.
    unchanged(&format!("    addq %rdx, %rsi\n    cmpq %rsi, %rdx\n{tail}"));
    // Doubling: the addend is the destination, which the add changed.
    unchanged(&format!("    addq %rsi, %rsi\n    cmpq %rsi, %rsi\n{tail}"));
    // The addend is addressed through the sum, which the add changed.
    unchanged(&format!(
        "    addq (%rsi), %rsi\n    cmpq (%rsi), %rsi\n{tail}"
    ));
    // Width mismatch.
    unchanged(&format!("    addl %edx, %esi\n    cmpq %rdx, %rsi\n{tail}"));
    // ZF is read: `add` and `cmp` disagree on it.
    unchanged(
        "    addq %rdx, %rsi\n    cmpq %rdx, %rsi\n    setbe %r10b\n    movzbl %r10b, %r10d\n    movq %r10, (%rdi)\n    ret",
    );
    unchanged("    addq %rdx, %rsi\n    cmpq %rdx, %rsi\n    jb .Lx\n    ret\n.Lx:\n    ret");
    // CF is read by `setb`, but the flags outlive it.
    unchanged("    addq %rdx, %rsi\n    cmpq %rdx, %rsi\n    setb %r10b\n    sete %r11b\n    ret");
    // Subtraction: the compare must be of the ORIGINAL minuend.
    unchanged(&format!(
        "    movq %r9, %rax\n    subq %rdx, %rax\n    cmpq %rdx, %rax\n{tail}"
    ));
    unchanged(&format!("    subq %rdx, %rax\n    cmpq %rdx, %r9\n{tail}"));
    // An `lea` with a scale, or whose sum is also an addend, is not rebuilt.
    unchanged(&format!(
        "    leaq (%rax,%rdx,2), %r15\n    cmpq %rdx, %r15\n{tail}"
    ));
    unchanged(&format!(
        "    leaq (%rax, %rdx), %rax\n    cmpq %rdx, %rax\n{tail}"
    ));
    unchanged(&format!(
        "    leaq 8(%rax), %rcx\n    cmpq $9, %rcx\n{tail}"
    ));
    // Something between the arithmetic and the compare.
    unchanged(&format!(
        "    addq %rdx, %rsi\n    movq %rcx, %rdx\n    cmpq %rdx, %rsi\n{tail}"
    ));
}

#[test]
fn the_producer_fold_alone_keeps_a_plain_carry_setcc() {
    // No add consumes the byte: only the redundant compare goes.
    let out = folded(
        "    addq %rdx, %rsi\n    cmpq %rdx, %rsi\n    setb %r10b\n    movzbl %r10b, %r10d\n    movq %r10, (%rdi)\n    ret",
        &[],
    );
    assert_eq!(
        lines(&out),
        [
            "addq %rdx, %rsi",
            "setb %r10b",
            "movzbl %r10b, %r10d",
            "movq %r10, (%rdi)",
            "ret"
        ]
    );
}

#[test]
fn the_low_half_of_a_product_comes_from_the_mul_that_computed_the_high_half() {
    // In place, either operand order, unsigned and signed high half.
    for mul in ["mulq", "imulq"] {
        for (imul, low) in [
            ("imulq %rcx, %rsi", "movq %rax, %rsi"),
            ("imulq %rsi, %rcx", "movq %rax, %rcx"),
        ] {
            let out = folded(
                &format!(
                    "    movq %rsi, %rax\n    {mul} %rcx\n    movq %rdx, %rbx\n    {imul}\n    ret"
                ),
                &[],
            );
            assert_eq!(
                lines(&out),
                [
                    "movq %rsi, %rax",
                    &format!("{mul} %rcx"),
                    "movq %rdx, %rbx",
                    low,
                    "ret"
                ]
            );
        }
    }
    // Into a third register, past the parked high half.
    let out = folded(
        "    movq %r13, %rax\n    mulq %rcx\n    movq %rdx, %r15\n    movq %r13, %r11\n    imulq %rcx, %r11\n    ret",
        &[],
    );
    assert_eq!(
        lines(&out),
        [
            "movq %r13, %rax",
            "mulq %rcx",
            "movq %rdx, %r15",
            "movq %rax, %r11",
            "ret"
        ]
    );
    // Into %rax itself: it already holds the low half.
    let out = folded(
        "    movq %r13, %rax\n    mulq %rcx\n    movq %rdx, 32(%rsp)\n    movq %r13, %rax\n    imulq %rcx, %rax\n    movq %rax, 24(%rsp)\n    ret",
        &[],
    );
    assert_eq!(
        lines(&out),
        [
            "movq %r13, %rax",
            "mulq %rcx",
            "movq %rdx, 32(%rsp)",
            "movq %rax, 24(%rsp)",
            "ret"
        ]
    );
    // A square.
    let out = folded(
        "    movq %rsi, %rax\n    mulq %rsi\n    movq %rdx, %rbx\n    imulq %rsi, %rsi\n    ret",
        &[],
    );
    assert_eq!(lines(&out)[3], "movq %rax, %rsi");
}

#[test]
fn a_product_whose_inputs_or_low_half_changed_is_recomputed() {
    // A factor is overwritten between the two multiplies.
    unchanged(
        "    movq %rsi, %rax\n    mulq %rcx\n    movq %r9, %rcx\n    imulq %rcx, %rsi\n    ret",
    );
    unchanged(
        "    movq %rsi, %rax\n    mulq %rcx\n    movq %rdx, %rsi\n    imulq %rcx, %rsi\n    ret",
    );
    // The low half is overwritten.
    unchanged(
        "    movq %rsi, %rax\n    mulq %rcx\n    movq %r9, %rax\n    imulq %rcx, %rsi\n    ret",
    );
    // Not the same factors.
    unchanged("    movq %rsi, %rax\n    mulq %rcx\n    imulq %r8, %rsi\n    ret");
    unchanged(
        "    movq %rsi, %rax\n    mulq %rcx\n    movq %r8, %r11\n    imulq %rcx, %r11\n    ret",
    );
    // Arithmetic in between is not a move this fold understands.
    unchanged(
        "    movq %rsi, %rax\n    mulq %rcx\n    addq $1, %r9\n    imulq %rcx, %rsi\n    ret",
    );
    // The `imul`'s overflow flag is read.
    unchanged(
        "    movq %rsi, %rax\n    mulq %rcx\n    imulq %rcx, %rsi\n    jo .Lx\n    ret\n.Lx:\n    ret",
    );
    // The accumulator was not loaded from a register just before the `mul`.
    unchanged(
        "    movq %rsi, %rax\n    movq %r9, %rcx\n    mulq %rcx\n    imulq %rcx, %rsi\n    ret",
    );
    // A 32-bit multiply is a different product.
    unchanged("    movq %rsi, %rax\n    mulq %rcx\n    imull %ecx, %esi\n    ret");
    // A label between: another path may arrive with a different %rax.
    unchanged("    movq %rsi, %rax\n    mulq %rcx\n.Ljoin:\n    imulq %rcx, %rsi\n    ret");
    // A factor in %rdx is destroyed by the multiply itself.
    unchanged("    movq %rsi, %rax\n    mulq %rdx\n    imulq %rdx, %rsi\n    ret");
}

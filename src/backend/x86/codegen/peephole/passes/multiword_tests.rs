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
        let e = forward_carry_flag(&mut store, &mut infos);
        if !(a || b || c || d || e) {
            break;
        }
        changed = true;
    }
    let out = (0..store.len())
        .filter(|&i| !infos[i].is_nop())
        .map(|i| store.get(i).trim_ascii())
        .filter(|line| !line.starts_with(".cfi") && *line != "foo:")
        .collect::<Vec<_>>()
        .join("\n");
    (changed, out)
}

/// Sixteen registers, 8-byte memory cells keyed by address (a 32-bit store
/// writes the low half of its cell), CF, and ZF where a loop is counted
/// (written by the instructions that count: `sub`, `add`, `dec`, `test`,
/// `xor`).
#[derive(Clone, Debug)]
struct Cpu {
    regs: [u64; 16],
    memory: FxHashMap<u64, u64>,
    carry: bool,
    zero: bool,
}

/// Two states are the same in what a fold has to keep: ZF is the model's
/// own, for the loops it runs.
impl PartialEq for Cpu {
    fn eq(&self, other: &Self) -> bool {
        (&self.regs, &self.memory, self.carry) == (&other.regs, &other.memory, other.carry)
    }
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
        let text = text.trim_ascii();
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
        let disp = match text[..open].trim_ascii() {
            "" => 0i64,
            literal => literal.parse().expect("displacement"),
        };
        let inner = text[open + 1..].strip_suffix(')').expect("closing paren");
        let mut parts = inner.split(',').map(str::trim_ascii);
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
                let cell = match size {
                    Size::Q => value,
                    Size::L => {
                        let high = self.read(&Place::Mem(*address), Size::Q) & !0xffff_ffff;
                        high | (value & 0xffff_ffff)
                    }
                    Size::B => panic!("model stores no bytes"),
                };
                self.memory.insert(*address, cell);
            }
            Place::Imm(_) => panic!("write to an immediate"),
        }
    }

    fn step(&mut self, line: &str) {
        let (mnem, args) = line.split_once(' ').unwrap_or((line, ""));
        let operands: Vec<&str> = match top_level_operands(args) {
            Some((a, b)) => vec![a, b],
            None => vec![args.trim_ascii()],
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
                self.zero = sum & limit == 0;
                self.write(&dst, size, sum as u64);
            }
            "subq" | "subl" | "sbbq" | "sbbl" | "cmpq" | "cmpl" => {
                let dst = self.place(operands[1]);
                let borrow_in = u128::from(mnem.starts_with("sbb") && self.carry);
                let minuend = u128::from(self.read(&dst, size));
                let subtrahend = u128::from(self.read(&self.place(operands[0]), size)) + borrow_in;
                self.carry = minuend < subtrahend;
                self.zero = minuend.wrapping_sub(subtrahend) & limit == 0;
                if !mnem.starts_with("cmp") {
                    self.write(&dst, size, minuend.wrapping_sub(subtrahend) as u64);
                }
            }
            // Counts down and leaves CF as it was.
            "decq" | "decl" => {
                let dst = self.place(operands[0]);
                let left = u128::from(self.read(&dst, size)).wrapping_sub(1) & limit;
                self.zero = left == 0;
                self.write(&dst, size, left as u64);
            }
            "testq" | "testl" => {
                let both = self.read(&self.place(operands[0]), size)
                    & self.read(&self.place(operands[1]), size);
                self.carry = false;
                self.zero = both == 0;
            }
            "xorl" | "xorq" => {
                let dst = self.place(operands[1]);
                let value = self.read(&dst, size) ^ self.read(&self.place(operands[0]), size);
                self.carry = false;
                self.zero = value == 0;
                self.write(&dst, size, value);
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
            // Two destinations each; neither writes a flag.
            "xchg" | "xchgq" => {
                let (a, b) = (self.place(operands[0]), self.place(operands[1]));
                let (x, y) = (self.read(&a, Size::Q), self.read(&b, Size::Q));
                self.write(&a, Size::Q, y);
                self.write(&b, Size::Q, x);
            }
            "mulxq" => {
                let (factor, low) = top_level_operands(operands[0]).expect("mulx operands");
                let product =
                    u128::from(self.regs[2]) * u128::from(self.read(&self.place(factor), Size::Q));
                // The high half is written last: it wins when both name
                // one register.
                let (low, high) = (self.place(low), self.place(operands[1]));
                self.write(&low, Size::Q, product as u64);
                self.write(&high, Size::Q, (product >> 64) as u64);
            }
            "btl" | "btq" => {
                let Place::Imm(bit) = self.place(operands[0]) else {
                    panic!("model tests constant bits only ({line})");
                };
                self.carry = (self.read(&self.place(operands[1]), size) >> bit) & 1 != 0;
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

    /// Every line of `body` that is an instruction; a directive is none.
    fn run(mut self, body: &str) -> Self {
        for line in body
            .lines()
            .map(str::trim_ascii)
            .filter(|l| !l.is_empty() && !l.starts_with('.'))
        {
            self.step(line);
        }
        self
    }

    /// `body` as a program: a label is a place, `jne`, `je` and `jmp` go to
    /// one, and `ret` or the last line ends it.
    fn run_program(mut self, body: &str) -> Self {
        let lines: Vec<&str> = body
            .lines()
            .map(str::trim_ascii)
            .filter(|l| !l.is_empty())
            .collect();
        let place_of = |label: &str| {
            lines
                .iter()
                .position(|line| line.strip_suffix(':') == Some(label))
                .unwrap_or_else(|| panic!("no label {label}"))
        };
        let (mut at, mut fuel) = (0, 10_000);
        while let Some(&line) = lines.get(at) {
            fuel -= 1;
            assert!(fuel > 0, "the program does not end:\n{body}");
            at += 1;
            if line.ends_with(':') || line.starts_with('.') {
                continue;
            }
            match line.split_once(' ') {
                Some(("jne", label)) if !self.zero => at = place_of(label),
                Some(("je", label)) if self.zero => at = place_of(label),
                Some(("jne" | "je", _)) => {}
                Some(("jmp", label)) => at = place_of(label),
                _ if line == "ret" => break,
                _ => self.step(line),
            }
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
    at.map(|i| (args[..i].trim_ascii(), args[i + 1..].trim_ascii()))
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
            zero: round % 3 == 0,
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
    text.lines().map(str::trim_ascii).collect()
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

/// The second limb of the unrolled row in `tests/bench/k_bn_mul128.c`, as
/// the register allocator left it: both carries pass through `%al`, and the
/// carry-out lands in `%rbp`.  The later carry folds first; that removes the
/// `setb %al` which kept `%rax` observable after the earlier one.
#[test]
fn chained_carries_through_one_byte_register_and_rbp_fold() {
    // Both carries go through `%al`, and the second lands in `%rbp`.  The
    // second carry has to fold first: until its `setb %al` is gone, the
    // first widened carry in `%rax` is not provably dead.  Every register
    // the folds release is redefined before the `ret`, as in the kernel.
    let out = folded(
        "    addq %r11, %rbp\n    cmpq %r11, %rbp\n    setb %al\n    movzbl %al, %eax\n    addq %rax, %rdx\n    leaq (%rbp, %r14, 1), %r11\n    cmpq %r14, %r11\n    setb %al\n    movzbl %al, %r14d\n    leaq (%rdx, %r14), %rbp\n    movq %r11, 8(%rdi)\n    movq %r15, %rax\n    movq %r15, %r14\n    ret",
        &[],
    );
    let text = lines(&out);
    assert!(text.contains(&"adcq $0, %rdx"), "{out}");
    assert!(text.contains(&"adcq $0, %rbp"), "{out}");
    assert!(
        !text.iter().any(|line| ["set", "cmp", "movzb", "lea"]
            .iter()
            .any(|stem| line.starts_with(stem))),
        "{out}"
    );
}

/// `%rbp` as the high word of every form: the folded instruction writes the
/// register the original one wrote.
#[test]
fn rbp_is_an_ordinary_high_word() {
    let out = folded(
        &format!("    addq %rdx, %rsi\n    cmpq %rdx, %rsi\n{CARRY}\n    addq %r10, %rbp\n    ret"),
        &["%r10"],
    );
    assert_eq!(lines(&out), ["addq %rdx, %rsi", "adcq $0, %rbp", "ret"]);
    let out = folded(
        &format!(
            "    leaq (%rax, %rdx), %rbp\n    cmpq %rdx, %rbp\n{CARRY}\n    addq %r10, %rbx\n    ret"
        ),
        &["%r10"],
    );
    assert_eq!(
        lines(&out),
        ["movq %rax, %rbp", "addq %rdx, %rbp", "adcq $0, %rbx", "ret"]
    );
    // The stack pointer is never the high word of these rewrites.
    let (_, out) = run_folds(&format!(
        "    addq %rdx, %rsi\n    cmpq %rdx, %rsi\n{CARRY}\n    addq %r10, %rsp\n    ret"
    ));
    assert!(lines(&out).contains(&"addq %r10, %rsp"), "{out}");
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

/// The shapes the 128-bit split leaves behind: the factor staged into a
/// scratch register, the high half summed (not just moved) before the low
/// half is asked for again, and a square spelled through the staged copy.
#[test]
fn a_staged_factor_and_arithmetic_in_between_still_share_the_mul() {
    // Square: `%rcx` is a copy of `%r10`, and the second multiply names the
    // original on both sides.
    let out = folded(
        "    movq %r10, %rax\n    movq %r10, %rcx\n    mulq %rcx\n    leaq (%rdx, %r9), %rbx\n    imulq %r10, %r10\n    ret",
        &[],
    );
    assert_eq!(
        lines(&out),
        [
            "movq %r10, %rax",
            "movq %r10, %rcx",
            "mulq %rcx",
            "leaq (%rdx, %r9), %rbx",
            "movq %rax, %r10",
            "ret"
        ]
    );
    // Two factors, the second staged; an `add` and an `adc` in between.
    let out = folded(
        "    movq %r9, %rax\n    movq %r8, %rcx\n    mulq %rcx\n    addq %rsi, %rdx\n    adcq $0, %rbx\n    imulq %r8, %r9\n    ret",
        &[],
    );
    assert_eq!(lines(&out)[5], "movq %rax, %r9");
    // The staged factor comes from memory: `%rcx` is its only name.
    let out = folded(
        "    movq %r9, %rax\n    movq 8(%rdi), %rcx\n    mulq %rcx\n    movq %rdx, %rbx\n    imulq %rcx, %r9\n    ret",
        &[],
    );
    assert_eq!(lines(&out)[4], "movq %rax, %r9");
}

/// The bignum step as a function of its own (`mac` in the corpus test):
/// the factor arrives in `%rdx`, is parked in `%r8`, staged into `%rcx`
/// from `%rdx`, and the low half is asked for again through `%r8`.
#[test]
fn a_factor_parked_before_it_was_staged_is_still_the_same_factor() {
    let out = folded(
        "    movq %rdx, %r8\n    movq %rcx, %r9\n    movq %rsi, %rax\n    movq %rdx, %rcx\n    mulq %rcx\n    imulq %r8, %rsi\n    addq (%rdi), %rsi\n    adcq $0, %rdx\n    ret",
        &[],
    );
    assert_eq!(lines(&out)[5], "movq %rax, %rsi");
}

#[test]
fn a_staged_factor_that_no_longer_names_the_value_is_not_shared() {
    for body in [
        // The parked copy was changed before the product.
        "    movq %rdx, %r8\n    addq $1, %r8\n    movq %rsi, %rax\n    movq %rdx, %rcx\n    mulq %rcx\n    imulq %r8, %rsi\n    ret",
        // The staged register was redefined after it was parked.
        "    movq %rdx, %r8\n    movq %r9, %rdx\n    movq %rsi, %rax\n    movq %rdx, %rcx\n    mulq %rcx\n    imulq %r8, %rsi\n    ret",
        // The staging move overwrote `%a` itself: `%r9 * %r9` is not
        // `old %r9 * %r8`.
        "    movq %r9, %rax\n    movq %r8, %r9\n    mulq %r9\n    movq %rdx, %rbx\n    imulq %r9, %r9\n    ret",
        // The alias was rewritten between the two multiplies.
        "    movq %r9, %rax\n    movq %r8, %rcx\n    mulq %rcx\n    addq $1, %r8\n    imulq %r8, %r9\n    ret",
        // `%rax` is not a name for the factor after the multiply.
        "    movq %r9, %rax\n    movq %rax, %rcx\n    mulq %rcx\n    movq %rdx, %rbx\n    imulq %rax, %r9\n    ret",
        // A line with a second output ends the window.
        "    movq %r9, %rax\n    mulq %r8\n    xchgq %rbx, %r8\n    imulq %r8, %r9\n    ret",
        // So does a divide: it rewrites the low half.
        "    movq %r9, %rax\n    mulq %r8\n    divq %rbx\n    imulq %r8, %r9\n    ret",
    ] {
        unchanged(body);
    }
}

/// What the allocator leaves between the two halves of a product is looked
/// past when it writes neither the low half nor a factor: a sum into the
/// high word, a parked limb, a store.
#[test]
fn unrelated_moves_and_sums_between_the_halves_are_looked_past() {
    for between in [
        "addq $1, %r9",
        "leaq (%rdx, %r9), %r10",
        "movq %rdx, %r11",
        "movq %rdx, 8(%rsp)",
        "subq %r9, %r10",
    ] {
        let out = folded(
            &format!(
                "    movq %rsi, %rax\n    mulq %rcx\n    {between}\n    imulq %rcx, %rsi\n    ret"
            ),
            &[],
        );
        let text = lines(&out);
        assert!(text.contains(&"movq %rax, %rsi"), "{out}");
        assert!(text.contains(&between), "{out}");
        assert!(!text.iter().any(|line| line.starts_with("imulq")), "{out}");
    }
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
    // Arithmetic that writes the low half or a factor.  (On any other
    // register it is looked past: see the test below.)
    for clobbered in ["%rax", "%rsi", "%rcx"] {
        unchanged(&format!(
            "    movq %rsi, %rax\n    mulq %rcx\n    addq $1, {clobbered}\n    imulq %rcx, %rsi\n    ret"
        ));
    }
    // A line with a second or an implicit output, or one that is not a
    // two-operand move or sum at all, ends the window whatever it writes.
    for opaque in ["xchgq %r8, %r9", "shlq $1, %r9", "negq %r9", "cqto"] {
        unchanged(&format!(
            "    movq %rsi, %rax\n    mulq %rcx\n    {opaque}\n    imulq %rcx, %rsi\n    ret"
        ));
    }
    // The `imul`'s overflow flag is read.
    unchanged(
        "    movq %rsi, %rax\n    mulq %rcx\n    imulq %rcx, %rsi\n    jo .Lx\n    ret\n.Lx:\n    ret",
    );
    // The accumulator is not a copy of a register when the `mul` runs: it
    // was loaded from memory, or changed after it was copied.  (A factor
    // staged between the copy and the `mul` is a different matter: see
    // `a_staged_factor_and_arithmetic_in_between_still_share_the_mul`.)
    unchanged("    movq (%rdi), %rax\n    mulq %rcx\n    imulq %rcx, %rsi\n    ret");
    unchanged(
        "    movq %rsi, %rax\n    addq $1, %rax\n    mulq %rcx\n    imulq %rcx, %rsi\n    ret",
    );
    // Two moves between the copy and the `mul` are not the staged shape.
    unchanged(
        "    movq %rsi, %rax\n    movq %r9, %rcx\n    movq %r8, %rsi\n    mulq %rcx\n    imulq %rcx, %rsi\n    ret",
    );
    // A 32-bit multiply is a different product.
    unchanged("    movq %rsi, %rax\n    mulq %rcx\n    imull %ecx, %esi\n    ret");
    // A label between: another path may arrive with a different %rax.
    unchanged("    movq %rsi, %rax\n    mulq %rcx\n.Ljoin:\n    imulq %rcx, %rsi\n    ret");
    // A factor in %rdx is destroyed by the multiply itself.
    unchanged("    movq %rsi, %rax\n    mulq %rdx\n    imulq %rdx, %rsi\n    ret");
}

/// One link of a chain as the full adder emits it: the carry in from
/// `carry` by `bt`, the sum of two limbs, the carry out into `out`.
fn link(offset: u32, carry: &str, out: &str, mnemonic: &str) -> String {
    format!(
        "    movq {offset}(%rsp), %rax\n    btl $0, {carry}\n    {mnemonic} {offset}(%rdi), %rax\n    \
         setc %cl\n    movzbl %cl, {out}\n    movq %rax, {offset}(%rsp)"
    )
}

#[test]
fn a_carry_still_in_the_flag_is_not_put_back() {
    for (first, then) in [("addq", "adcq"), ("subq", "sbbq")] {
        // Four limbs in memory: one `add`, three `adc`, and no boolean.
        let body = format!(
            "    movq (%rsp), %rax\n    {first} (%rdi), %rax\n    setc %cl\n    movzbl %cl, %r10d\n    \
             movq %rax, (%rsp)\n{}\n{}\n{}\n    movl %r9d, %eax\n    ret",
            link(8, "%r10d", "%r8d", then),
            link(16, "%r8d", "%r11d", then),
            link(24, "%r11d", "%r9d", then),
        );
        let out = folded(&body, &["%rcx", "%r10", "%r8", "%r11"]);
        let text = lines(&out);
        assert!(!text.iter().any(|line| line.starts_with("bt")), "{out}");
        // The last carry is returned: its `setc` and extension stay.
        assert_eq!(
            text.iter().filter(|line| line.starts_with("setc")).count(),
            1,
            "{out}"
        );
        assert_eq!(
            text.iter().filter(|line| line.starts_with(then)).count(),
            3,
            "{out}"
        );
        assert_eq!(
            text,
            lines(&format!(
                "movq (%rsp), %rax\n{first} (%rdi), %rax\nmovq %rax, (%rsp)\n\
                 movq 8(%rsp), %rax\n{then} 8(%rdi), %rax\nmovq %rax, 8(%rsp)\n\
                 movq 16(%rsp), %rax\n{then} 16(%rdi), %rax\nmovq %rax, 16(%rsp)\n\
                 movq 24(%rsp), %rax\n{then} 24(%rdi), %rax\nsetc %cl\nmovzbl %cl, %r9d\n\
                 movq %rax, 24(%rsp)\nmovl %r9d, %eax\nret"
            )),
            "{out}"
        );
    }
}

#[test]
fn a_carry_that_is_read_again_keeps_its_boolean() {
    // The boolean is added up as well: only the `bt` goes.
    let out = folded(
        "    addq %rsi, %rax\n    setc %cl\n    movzbl %cl, %r10d\n    movq %rdx, %r8\n    \
         btl $0, %r10d\n    adcq %r9, %r8\n    movq %r10, %rsi\n    ret",
        &["%rcx"],
    );
    assert_eq!(
        lines(&out),
        [
            "addq %rsi, %rax",
            "setc %cl",
            "movzbl %cl, %r10d",
            "movq %rdx, %r8",
            "adcq %r9, %r8",
            "movq %r10, %rsi",
            "ret"
        ]
    );
    // The byte is its own extension (`setc %al; movzbl %al, %eax`), and a
    // 64-bit test of it.
    let out = folded(
        "    addq %rsi, %rdx\n    setc %r10b\n    movzbq %r10b, %r10\n    movq %rdi, %r8\n    \
         btq $0, %r10\n    sbbq %r9, %r8\n    ret",
        &["%r10"],
    );
    assert_eq!(
        lines(&out),
        ["addq %rsi, %rdx", "movq %rdi, %r8", "sbbq %r9, %r8", "ret"]
    );
}

#[test]
fn a_carry_the_flag_no_longer_holds_is_tested() {
    let chain = |between: &str, set: &str, bit: &str| {
        format!(
            "    addq %rsi, %rax\n    {set} %cl\n    movzbl %cl, %r10d\n{between}    movq %rdx, %r8\n    \
             btl {bit}, %r10d\n    adcq %r9, %r8\n    ret"
        )
    };
    // The reference shape folds.
    folded(&chain("", "setc", "$0"), &["%rcx", "%r10"]);
    // A flag was written in between: the counter of a loop, a compare.
    unchanged(&chain("    addq $1, %r11\n", "setc", "$0"));
    unchanged(&chain("    cmpq %r11, %rdi\n", "setc", "$0"));
    // The boolean was written in between, in whole or in its low byte.
    unchanged(&chain("    movq %r11, %r10\n", "setc", "$0"));
    unchanged(&chain("    movb %r11b, %r10b\n", "setc", "$0"));
    // Not the carry, or not its bit.
    unchanged(&chain("", "setnc", "$0"));
    unchanged(&chain("", "sete", "$0"));
    unchanged(&chain("", "setc", "$1"));
    // Another path may arrive with a different flag; a call returns with
    // any flag.
    unchanged(&chain(".Ljoin:\n", "setc", "$0"));
    unchanged(&chain("    call helper\n", "setc", "$0"));
    // The byte was written between the `setc` and its extension.
    unchanged(
        "    addq %rsi, %rax\n    setc %cl\n    movb %r11b, %cl\n    movzbl %cl, %r10d\n    \
         btl $0, %r10d\n    adcq %r9, %r8\n    ret",
    );
    // A boolean that is no extension of a `setc` at all.
    unchanged(
        "    addq %rsi, %rax\n    movq %rdi, %r10\n    btl $0, %r10d\n    adcq %r9, %r8\n    ret",
    );
    // A variable bit.
    unchanged(
        "    addq %rsi, %rax\n    setc %cl\n    movzbl %cl, %r10d\n    btl %edi, %r10d\n    \
         adcq %r9, %r8\n    ret",
    );
}

/// [`widen_carry_in_place`] alone, once, as the pipeline runs it.
fn widen(body: &str) -> (bool, String) {
    let text = format!("foo:\n.cfi_startproc\n{body}\n.cfi_endproc\n");
    let mut store = LineStore::new(text);
    let mut infos: Vec<_> = (0..store.len())
        .map(|i| classify_line(store.get(i)))
        .collect();
    let changed = widen_carry_in_place(&mut store, &mut infos);
    let out = (0..store.len())
        .filter(|&i| !infos[i].is_nop())
        .map(|i| store.get(i).trim_ascii())
        .filter(|line| !line.starts_with(".cfi") && *line != "foo:")
        .collect::<Vec<_>>()
        .join("\n");
    (changed, out)
}

/// Widen in place and require the same registers (all but `scratch`) and
/// memory from every state.
fn widened(body: &str, scratch: &[&str]) -> String {
    let (changed, out) = widen(body);
    assert!(changed, "not widened:\n{body}");
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

fn not_widened(body: &str) {
    let (changed, out) = widen(body);
    assert!(!changed, "must stay:\n{body}\n=>\n{out}");
}

#[test]
fn a_carry_out_goes_straight_into_its_register() {
    // Every producer the full adder emits, both widths of the extension.
    for producer in ["addq", "adcq", "subq", "sbbq"] {
        for extension in ["movzbl %cl, %r11d", "movzbq %cl, %r11"] {
            let body = format!(
                "    movq (%rsp), %rax\n    btl $0, %r11d\n    {producer} (%rdi), %rax\n    setc %cl\n    \
                 {extension}\n    movq %rax, (%rsp)\n    movq %r11, %rax\n    ret"
            );
            let out = widened(&body, &["%rcx"]);
            assert_eq!(
                lines(&out),
                lines(&format!(
                    "movq (%rsp), %rax\nbtl $0, %r11d\n{producer} (%rdi), %rax\nmovl $0, %r11d\n\
                     setc %r11b\nmovq %rax, (%rsp)\nmovq %r11, %rax\nret"
                )),
                "{out}"
            );
        }
    }
    // The word stored between the two: the store stays where it is.
    let out = widened(
        "    adcq 8(%rdi), %rax\n    setc %cl\n    movq %rax, 8(%rsp)\n    movzbl %cl, %r9d\n    \
         movl %r9d, %eax\n    ret",
        &["%rcx"],
    );
    assert_eq!(
        lines(&out),
        lines(
            "adcq 8(%rdi), %rax\nmovl $0, %r9d\nmovq %rax, 8(%rsp)\nsetc %r9b\nmovl %r9d, %eax\nret"
        ),
        "{out}"
    );
    // The byte and its extension in one register: nothing to prove dead.
    let out = widened(
        "    sbbq %rsi, %rdx\n    setb %al\n    movzbl %al, %eax\n    ret",
        &[],
    );
    assert_eq!(
        lines(&out),
        lines("sbbq %rsi, %rdx\nmovl $0, %eax\nsetc %al\nret"),
        "{out}"
    );
    // Two links that both keep their carry: one scratch byte for the two,
    // and neither keeps it.
    let out = widened(
        "    adcq 8(%rdi), %rax\n    setc %cl\n    movzbl %cl, %r12d\n    movq %rax, 8(%rsp)\n    \
         movq 16(%rsp), %rax\n    adcq 16(%rdi), %rax\n    setc %cl\n    movzbl %cl, %r13d\n    \
         leaq (%r12,%r13), %rax\n    ret",
        &["%rcx"],
    );
    assert!(!out.contains("%cl"), "{out}");
    // The low bytes that need a REX prefix have their own names.
    let out = widened(
        "    adcq %rdx, %rax\n    setc %cl\n    movzbl %cl, %esi\n    movq %rsi, (%rsp)\n    ret",
        &["%rcx"],
    );
    assert!(out.contains("movl $0, %esi\nsetc %sil"), "{out}");
}

#[test]
fn a_carry_round_a_loop_goes_straight_into_its_register() {
    // The scratch byte is written again every trip, which the liveness
    // reads as a use of the rest of %rcx: the question has to be asked of
    // the rewritten loop.  (%rcx carries the bound into the function and is
    // copied out of the way, so it is not private to the pair either.)
    let (changed, out) = widen(
        "    movq %rcx, %r8\n    movl $0, %r9d\n    movl $0, %r11d\n.Lhead:\n    \
         movq (%rsi,%r9,8), %rax\n    btl $0, %r11d\n    adcq (%rdx,%r9,8), %rax\n    setc %cl\n    \
         movzbl %cl, %r11d\n    movq %rax, (%rdi,%r9,8)\n    addq $1, %r9\n    cmpq %r8, %r9\n    \
         jl .Lhead\n    movq %r11, %rax\n    ret",
    );
    assert!(changed, "{out}");
    assert!(
        out.contains(
            "adcq (%rdx,%r9,8), %rax\nmovl $0, %r11d\nsetc %r11b\nmovq %rax, (%rdi,%r9,8)"
        ),
        "{out}"
    );
}

#[test]
fn two_carries_a_trip_both_go_straight_into_their_registers() {
    // Each link's scratch byte is the other's, round the back edge: the
    // lower one is settled by a second sweep.
    let (changed, out) = widen(
        "    movq %rcx, %r12\n    movl $0, %r9d\n    movl $0, %r10d\n.Lhead:\n    \
         movq (%rsi,%r10,8), %rax\n    btl $0, %r9d\n    adcq (%rdx,%r10,8), %rax\n    setc %cl\n    \
         movzbl %cl, %r8d\n    movq %rax, (%rdi,%r10,8)\n    addl %r8d, %r11d\n    \
         movq 8(%rsi,%r10,8), %rax\n    btl $0, %r8d\n    adcq 8(%rdx,%r10,8), %rax\n    setc %cl\n    \
         movzbl %cl, %r9d\n    movq %rax, 8(%rdi,%r10,8)\n    addq $2, %r10\n    cmpq %r12, %r10\n    \
         jl .Lhead\n    leal (%r11,%r9), %eax\n    ret",
    );
    assert!(changed, "{out}");
    assert!(!out.contains("%cl"), "{out}");
    assert_eq!(out.matches("movl $0, %r8d\nsetc %r8b").count(), 1, "{out}");
    assert_eq!(out.matches("movl $0, %r9d\nsetc %r9b").count(), 1, "{out}");
}

#[test]
fn a_carry_out_that_cannot_move_keeps_its_scratch_byte() {
    let stays = |what: &str, body: &str| {
        let (changed, out) = widen(body);
        assert!(!changed, "{what}:\n{body}\n=>\n{out}");
    };
    stays(
        "the scratch byte is read again",
        "    adcq %rdx, %rax\n    setc %cl\n    movzbl %cl, %r9d\n    movzbl %cl, %r10d\n    \
         movq %r10, (%rsp)\n    movl %r9d, %eax\n    ret",
    );
    stays(
        "the whole scratch register is read",
        "    adcq %rdx, %rax\n    setc %cl\n    movzbl %cl, %r9d\n    movq %rcx, (%rsp)\n    \
         movl %r9d, %eax\n    ret",
    );
    stays(
        "the old value of the register is read in between",
        "    adcq %rdx, %rax\n    setc %cl\n    movq %r9, (%rsp)\n    movzbl %cl, %r9d\n    \
         movl %r9d, %eax\n    ret",
    );
    stays(
        "the old value addresses the store in between",
        "    adcq %rdx, %rax\n    setc %cl\n    movq %rax, (%rsp,%r9,8)\n    movzbl %cl, %r9d\n    \
         movl %r9d, %eax\n    ret",
    );
    stays(
        "the flags are written in between",
        "    adcq %rdx, %rax\n    setc %cl\n    addq $1, %r8\n    movzbl %cl, %r9d\n    \
         movl %r9d, %eax\n    ret",
    );
    stays(
        "the carry is a compare's",
        "    cmpq %rsi, %rdx\n    setb %cl\n    movzbl %cl, %r9d\n    movl %r9d, %eax\n    ret",
    );
    stays(
        "not the carry",
        "    adcq %rdx, %rax\n    setae %cl\n    movzbl %cl, %r9d\n    movl %r9d, %eax\n    ret",
    );
    stays(
        "a label between the addition and the setc",
        "    adcq %rdx, %rax\n.Lin:\n    setc %cl\n    movzbl %cl, %r9d\n    movl %r9d, %eax\n    ret",
    );
    stays(
        "the byte is not extended",
        "    adcq %rdx, %rax\n    setc %cl\n    movb %cl, (%rsp)\n    ret",
    );
    not_widened("    adcq %rdx, %rax\n    setc %cl\n    ret");
}

#[test]
fn a_sum_stored_before_its_carry_is_read_keeps_the_flag() {
    // `c[0] += lo; hi += c[0] < lo; c[1] += hi; c[2] += c[1] < hi`: the
    // store of each sum stands between the addition and the compare.
    let out = folded(
        "    movq (%rsp), %r8\n    addq %rsi, %r8\n    movq %r8, (%rsp)\n    cmpq %rsi, %r8\n    \
         adcq $0, %rdx\n    movq 8(%rsp), %r10\n    addq %rdx, %r10\n    movq %r10, 8(%rsp)\n    \
         cmpq %rdx, %r10\n    movq 16(%rsp), %rdx\n    adcq $0, %rdx\n    movq %rdx, 16(%rsp)\n    ret",
        &[],
    );
    assert!(!out.contains("cmpq"), "{out}");
    // A borrow: `d = a - x` stored, then `a < x`.
    let out = folded(
        "    movq %r9, %r8\n    subq %rsi, %r8\n    movq %r8, (%rsp)\n    cmpq %rsi, %r9\n    \
         sbbq $0, %rdx\n    movq %rdx, %rax\n    ret",
        &[],
    );
    assert!(!out.contains("cmpq"), "{out}");
    // The other addend, which the sum's register held before the addition.
    let out = folded(
        "    movq %r9, %r8\n    addq (%rdi), %r8\n    movq %r8, (%rdi)\n    cmpq %r9, %r8\n    \
         adcq $0, %rdx\n    movq %rdx, %rax\n    ret",
        &[],
    );
    assert!(!out.contains("cmpq"), "{out}");

    // An addend written in between is not the addend any more.
    unchanged(
        "    addq %rsi, %r8\n    movq (%rsp), %rsi\n    cmpq %rsi, %r8\n    adcq $0, %rdx\n    \
         movq %rdx, %rax\n    ret",
    );
    // Nor is the sum.
    unchanged(
        "    addq %rsi, %r8\n    movq (%rsp), %r8\n    cmpq %rsi, %r8\n    adcq $0, %rdx\n    \
         movq %rdx, %rax\n    ret",
    );
    // An addend in memory may be what the store in between wrote.
    unchanged(
        "    addq (%rdi), %r8\n    movq %r8, (%rdi)\n    cmpq (%rdi), %r8\n    adcq $0, %rdx\n    \
         movq %rdx, %rax\n    ret",
    );
    // Something else wrote the flags in between.
    unchanged(
        "    addq %rsi, %r8\n    movq %r8, (%rsp)\n    subq $1, %r11\n    movq %r11, 8(%rsp)\n    \
         cmpq %rsi, %r8\n    adcq $0, %rdx\n    movq %rdx, %rax\n    ret",
    );
}

#[test]
fn a_second_destination_between_the_lines_is_a_write() {
    // `xchg` writes both its operands and `mulx` two of its three: what
    // the compare names is another value by then, and the compare is a
    // comparison of its own, not the carry of the addition above it.
    let pair = |between: &str| {
        format!(
            "    addq %r9, %r8\n    {between}\n    cmpq %r9, %r8\n    adcq $0, %r11\n    \
             movq %r11, %rax\n    ret"
        )
    };
    for between in [
        "xchg %r9, %rbx",
        "xchg %r8, %rbx",
        "xchg %rbx, %r9",
        "xchg %r9, (%rdi)",
        "mulxq %r12, %r9, %rbx",
        "mulxq %r12, %rbx, %r8",
    ] {
        unchanged(&pair(between));
    }
    // The same lines, naming neither register, are looked past.
    for between in ["xchg %r10, %rbx", "mulxq %r12, %r10, %rbx"] {
        let out = folded(&pair(between), &[]);
        assert!(!out.contains("cmpq"), "{out}");
    }

    // The boolean a `bt` tests, likewise: swapped for another value behind
    // its extension, or its byte swapped before it was extended.
    let chain = |early: &str, late: &str| {
        format!(
            "    addq %rsi, %rax\n    setc %cl\n{early}    movzbl %cl, %r10d\n{late}    \
             movq %rdx, %r8\n    btl $0, %r10d\n    adcq %r9, %r8\n    ret"
        )
    };
    folded(&chain("", ""), &["%rcx", "%r10"]);
    unchanged(&chain("", "    xchg %r10, %rbx\n"));
    unchanged(&chain("", "    xchg %rbx, %r10\n"));
    unchanged(&chain("", "    mulxq %r12, %r10, %r11\n"));
    unchanged(&chain("    xchg %rcx, %rbx\n", ""));
    unchanged(&chain("    xchg %cl, (%rdi)\n", ""));
    // Two other registers swapped behind the extension change neither.
    folded(&chain("", "    xchg %rbx, %r11\n"), &["%rcx", "%r10"]);
}

// ── A carry parked in the frame ──────────────────────────────────────────

/// Two links of a chain whose carry found no register: the first parks
/// its carry out in the frame (`park`), the second tests the slot
/// (`test`); `between` stands between the two.
fn parked(first: &str, then: &str, park: &str, between: &str, test: &str) -> String {
    format!(
        "    movq (%rdi), %rax\n    {first} 8(%rdi), %rax\n    setc %cl\n    movq %rax, 16(%rdi)\n    \
         movzbl %cl, %eax\n    {park}\n{between}    movq 24(%rdi), %rax\n    {test}\n    \
         {then} 32(%rdi), %rax\n    movq %rax, 40(%rdi)\n    ret"
    )
}

#[test]
fn a_carry_parked_in_the_frame_is_not_put_back() {
    let between = [
        "",
        // Another slot loaded, another slot stored.
        "    movq 16(%rsp), %r8\n",
        "    movq %r9, 24(%rsp)\n    movl %r9d, 32(%rsp)\n",
        // A store that ends where the tested byte begins, and one that
        // begins behind it.
        "    movl %r9d, 4(%rsp)\n",
        "    movq %r9, (%rsp)\n",
        "    movq %r9, 9(%rsp)\n",
        // Through a pointer: the limb of the trip, and `%rbp`, which is a
        // pointer where `%rsp` is the frame's base.
        "    movq %r9, 48(%rdi)\n    movq %r9, (%rbp)\n    movq 8(%rbp), %r8\n",
    ];
    for (first, then) in [("addq", "adcq"), ("subq", "sbbq")] {
        for (park, test) in [
            ("movl %eax, 8(%rsp)", "btl $0, 8(%rsp)"),
            ("movq %rax, 8(%rsp)", "btl $0, 8(%rsp)"),
            ("movq %rax, 8(%rsp)", "btq $0, 8(%rsp)"),
            ("movl %eax, (%rsp)", "btl $0, (%rsp)"),
        ] {
            for between in between {
                // The store at `(%rsp)` and the slot `(%rsp)` are one.
                if park.ends_with(", (%rsp)") && between.contains(", (%rsp)") {
                    continue;
                }
                let body = parked(first, then, park, between, test);
                let out = folded(&body, &[]);
                // The test is gone and nothing else: the boolean is still
                // stored, and the slot still holds it.
                let expected: Vec<&str> = lines(&body)
                    .into_iter()
                    .filter(|line| *line != test)
                    .collect();
                assert_eq!(lines(&out), expected, "{body}");
            }
        }
    }
}

#[test]
fn a_parked_carry_that_may_have_changed_is_tested() {
    let chain = |between: &str| {
        parked(
            "addq",
            "adcq",
            "movl %eax, 8(%rsp)",
            between,
            "btl $0, 8(%rsp)",
        )
    };
    folded(&chain(""), &[]);
    for between in [
        // The slot written again, in whole or over its first byte.
        "    movl %r9d, 8(%rsp)\n",
        "    movq %r9, 8(%rsp)\n",
        "    movq %r9, 4(%rsp)\n",
        "    movq %r9, 1(%rsp)\n",
        "    movl %r9d, 5(%rsp)\n",
        // Written by something that is not a plain move of 32 or 64 bits.
        "    movb %r9b, 8(%rsp)\n",
        "    movw %r9w, 16(%rsp)\n",
        "    movss %xmm0, 16(%rsp)\n",
        "    xchg %r9, 16(%rsp)\n",
        "    notq 16(%rsp)\n",
        // Read by something that is not a plain move.
        "    movzbl 16(%rsp), %r8d\n",
        // The frame's base moved: the same text names another slot.
        "    pushq %r9\n",
        "    popq %r9\n",
        "    leaq -16(%rsp), %rsp\n",
        "    movq %r9, %rsp\n",
        // A flag written, another path, a call.
        "    addq $1, %r11\n",
        "    cmpq %r11, %r9\n",
        ".Ljoin:\n",
        "    call helper\n",
        "    jmp .Lelsewhere\n",
    ] {
        unchanged(&chain(between));
    }

    // An address in the frame, anywhere in the function: what is stored
    // through a pointer may then be stored into the slot.
    for taken in [
        "    leaq 32(%rsp), %r8\n",
        "    leaq (%rsp), %r8\n",
        "    leaq 8(%rsp,%r9,8), %r8\n",
        "    movq 32(%rsp,%r9,8), %r8\n",
        "    movq %r9, (%rsp,%r8)\n",
        "    movq %rsp, %r8\n",
        "    movq %rsp, 32(%rsp)\n",
        "    addq %rsp, %r8\n",
        // A directive that emits data may be any of these (this one is
        // `mov %rsp, %rax`), and one that switches sections ends the
        // function's text.
        "    .byte 0x48, 0x89, 0xe0\n",
        "    .long 0\n",
        "    .section .text.unlikely\n",
        "    .p2align 4, 0xcc\n",
    ] {
        unchanged(&format!("{taken}{}", chain("")));
        let behind = chain("").replace("    ret", &format!("{taken}    ret"));
        unchanged(&behind);
    }
    // Directives that emit nothing, in front of the chain and behind it,
    // refuse nothing.
    for inert in [
        "    .p2align 4\n",
        "    .loc 1 12 4\n",
        "    .cfi_def_cfa_offset 16\n",
    ] {
        folded(&format!("{inert}{}", chain("")), &[]);
        let behind = chain("").replace("    ret", &format!("{inert}    ret"));
        folded(&behind, &[]);
    }

    // A frame addressed through `%rbp`.
    unchanged(&parked(
        "addq",
        "adcq",
        "movl %eax, -8(%rbp)",
        "",
        "btl $0, -8(%rbp)",
    ));
    // Another slot than the one that was parked in, another bit.
    unchanged(&parked(
        "addq",
        "adcq",
        "movl %eax, 8(%rsp)",
        "",
        "btl $0, 16(%rsp)",
    ));
    unchanged(&parked(
        "addq",
        "adcq",
        "movl %eax, 8(%rsp)",
        "",
        "btl $1, 8(%rsp)",
    ));
    // What was parked is not the carry: another register, a carry whose
    // flag was written since, the complement.
    unchanged(&parked(
        "addq",
        "adcq",
        "movl %r9d, 8(%rsp)",
        "",
        "btl $0, 8(%rsp)",
    ));
    unchanged(
        "    addq 8(%rdi), %rax\n    setc %cl\n    movzbl %cl, %eax\n    addq $1, %r11\n    \
         movl %eax, 8(%rsp)\n    movq 24(%rdi), %rax\n    btl $0, 8(%rsp)\n    \
         adcq 32(%rdi), %rax\n    ret",
    );
    unchanged(
        "    addq 8(%rdi), %rax\n    setnc %cl\n    movzbl %cl, %eax\n    movl %eax, 8(%rsp)\n    \
         movq 24(%rdi), %rax\n    btl $0, 8(%rsp)\n    adcq 32(%rdi), %rax\n    ret",
    );
    // An immediate parked.
    unchanged(
        "    addq 8(%rdi), %rax\n    movl $1, 8(%rsp)\n    movq 24(%rdi), %rax\n    \
         btl $0, 8(%rsp)\n    adcq 32(%rdi), %rax\n    ret",
    );
}

/// What the pipeline does with a parked carry: the test dropped
/// ([`forward_carry_flag`]), the store into a slot nobody reads any more
/// dropped (`eliminate_never_read_stores`, here by hand: `park`), and then
/// the boolean nobody reads ([`drop_unread_carries`]).
fn unparked(body: &str, park: &str) -> (bool, String) {
    let text = format!("foo:\n.cfi_startproc\n{body}\n.cfi_endproc\n");
    let mut store = LineStore::new(text);
    let mut infos: Vec<_> = (0..store.len())
        .map(|i| classify_line(store.get(i)))
        .collect();
    forward_carry_flag(&mut store, &mut infos);
    for i in 0..store.len() {
        if store.get(i).trim_ascii() == park {
            mark_nop(&mut infos[i]);
        }
    }
    let changed = drop_unread_carries(&mut store, &mut infos);
    let out = (0..store.len())
        .filter(|&i| !infos[i].is_nop())
        .map(|i| store.get(i).trim_ascii())
        .filter(|line| !line.starts_with(".cfi") && *line != "foo:")
        .collect::<Vec<_>>()
        .join("\n");
    (changed, out)
}

#[test]
fn a_carry_taken_out_of_the_flag_for_nobody_is_not_taken_out() {
    let park = "movl %eax, 8(%rsp)";
    for (first, then) in [("addq", "adcq"), ("subq", "sbbq")] {
        let body = parked(first, then, park, "", "btl $0, 8(%rsp)");
        let (changed, out) = unparked(&body, park);
        assert!(changed, "{body}");
        assert_eq!(
            lines(&out),
            lines(&format!(
                "movq (%rdi), %rax\n{first} 8(%rdi), %rax\nmovq %rax, 16(%rdi)\n\
                 movq 24(%rdi), %rax\n{then} 32(%rdi), %rax\nmovq %rax, 40(%rdi)\nret"
            )),
            "{body}"
        );
        // The same sums and the same registers but the scratch byte's, in
        // every memory cell but the slot.
        for state in states() {
            let before = state.clone().run(&body);
            let mut after = state.clone().run(&out);
            after.regs[1] = before.regs[1];
            let slot = 0x7000_0008;
            match before.memory.get(&slot) {
                Some(parked) => after.memory.insert(slot, *parked),
                None => after.memory.remove(&slot),
            };
            assert_eq!(before, after, "{body}\n=>\n{out}");
        }
    }

    let alone = |body: &str| {
        let text = format!("foo:\n.cfi_startproc\n{body}\n.cfi_endproc\n");
        let mut store = LineStore::new(text);
        let mut infos: Vec<_> = (0..store.len())
            .map(|i| classify_line(store.get(i)))
            .collect();
        let changed = drop_unread_carries(&mut store, &mut infos);
        let left = (0..store.len()).filter(|&i| !infos[i].is_nop()).count();
        (changed, store.len() - left)
    };
    // A boolean that is read stays: stored, returned, added up, or its
    // byte read.
    for reader in [
        "    movl %eax, 8(%rsp)\n    movq 24(%rdi), %rax\n",
        "",
        "    addq %rax, %r8\n    movq 24(%rdi), %rax\n",
        "    movq %rax, %r9\n    movq 24(%rdi), %rax\n",
    ] {
        let body = format!(
            "    addq 8(%rdi), %rax\n    setc %cl\n    movq %rax, 16(%rdi)\n    movzbl %cl, %eax\n\
             {reader}    ret"
        );
        assert_eq!(alone(&body), (false, 0), "{body}");
    }
    // The extension is unread and the byte is read: the `setc` stays.
    let body = "    addq 8(%rdi), %rax\n    setc %cl\n    movzbl %cl, %eax\n    movq 24(%rdi), %rax\n    \
                movq %rcx, %r9\n    ret";
    assert_eq!(alone(body), (true, 1), "{body}");
    // Neither is read: both go, the byte's own extension included.
    for pair in [
        "    setc %cl\n    movzbl %cl, %eax\n",
        "    setc %al\n    movzbl %al, %eax\n",
        "    setb %r10b\n    movzbq %r10b, %r10\n    movq %r9, %r10\n",
    ] {
        let body = format!(
            "    addq 8(%rdi), %rax\n{pair}    movq 24(%rdi), %rax\n    movq %rax, %rcx\n    ret"
        );
        assert_eq!(alone(&body), (true, 2), "{body}");
    }
    // Not a carry, or a flag written between the `setc` and its
    // extension: not this pass's.
    for pair in [
        "    sete %cl\n    movzbl %cl, %eax\n",
        "    setc %cl\n    addq $1, %r11\n    movzbl %cl, %eax\n",
        "    setc %cl\n.Ljoin:\n    movzbl %cl, %eax\n",
    ] {
        let body = format!(
            "    addq 8(%rdi), %rax\n{pair}    movq 24(%rdi), %rax\n    movq %rax, %rcx\n    ret"
        );
        assert_eq!(alone(&body), (false, 0), "{body}");
    }
}

// ── The carry round a loop ───────────────────────────────────────────────

fn kept(body: &str) -> (bool, String) {
    kept_unless(body, |_| {})
}

/// [`kept`], with `mark` applied to the line infos first (a pin, a
/// volatile access).
fn kept_unless(body: &str, mark: impl Fn(&mut [LineInfo])) -> (bool, String) {
    let (changed, out) = kept_with_directives(body, mark);
    let out = out
        .lines()
        .filter(|line| !line.starts_with(".cfi"))
        .collect::<Vec<_>>()
        .join("\n");
    (changed, out)
}

/// [`kept_unless`], with the unwind directives of `body` where the pass
/// left them. The store and the infos stay one line for one line, whatever
/// the pass added.
fn kept_with_directives(body: &str, mark: impl Fn(&mut [LineInfo])) -> (bool, String) {
    let text = format!("foo:\n.cfi_startproc\n{body}\n.cfi_endproc\n");
    let mut store = LineStore::new(text);
    let mut infos: Vec<_> = (0..store.len())
        .map(|i| classify_line(store.get(i)))
        .collect();
    let as_read: Vec<LineKind> = infos.iter().map(|info| info.kind).collect();
    mark(&mut infos);
    let marked = infos
        .iter()
        .zip(&as_read)
        .any(|(info, kind)| info.kind != *kind);
    let changed = keep_carry_round_loop(&mut store, &mut infos);
    assert_eq!(store.len(), infos.len(), "a line without its info");
    // Every line is what its info says it is, the added one too (unless
    // the test said otherwise of a line itself).
    for (i, info) in infos.iter().enumerate() {
        if !info.is_nop() && !marked {
            assert_eq!(
                info.kind,
                classify_line(store.get(i)).kind,
                "line {i} is not what its info says: {}",
                store.get(i)
            );
        }
    }
    let out = (0..store.len())
        .filter(|&i| !infos[i].is_nop())
        .map(|i| store.get(i).trim_ascii())
        .filter(|line| !matches!(*line, "foo:" | ".cfi_startproc" | ".cfi_endproc"))
        .collect::<Vec<_>>()
        .join("\n");
    (changed, out)
}

/// `body` keeps its carry in the flag and computes, from every state, the
/// registers (all but `%rax`, the loop's scratch) and the memory it did.
/// The flags are dead behind it.
fn kept_the_same(body: &str) -> String {
    let (changed, out) = kept(body);
    assert!(changed, "not kept:\n{body}");
    for state in states() {
        let before = state.clone().run_program(body);
        let after = state.clone().run_program(&out);
        for fam in 1..16 {
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

/// A limb loop as `limb_unroll` and the passes above leave it: `trips`
/// trips of two limbs through `%rdx`, `%r9` and `%r10`, the carry in
/// `carry` (a 64-bit register name without `%`), with `top` behind the
/// label, `adder` the mnemonic, `bottom` between the carry out and the
/// count, `count` the count, and `behind` behind the loop. `%rax` is its
/// scratch.
struct Loop<'a> {
    carry: &'a str,
    way_in: &'a str,
    top: &'a str,
    adder: &'a str,
    bottom: &'a str,
    count: &'a str,
    jump: &'a str,
    behind: &'a str,
}

impl Loop<'_> {
    fn limbs() -> Self {
        Loop {
            carry: "r8",
            way_in: "",
            top: "    movq (%rdx), %rax\n",
            adder: "adcq",
            bottom: "    movq %rax, 8(%r10)\n    leaq 16(%r10), %r10\n    leaq 16(%r9), %r9\n    \
                     leaq 16(%rdx), %rdx\n",
            count: "    subq $1, %r11\n    testq %r11, %r11\n",
            jump: "    jne .LBB7\n",
            behind: "    movq %r8, %rcx\n    xorl %eax, %eax\n    ret",
        }
    }

    fn text(&self) -> String {
        let Loop {
            carry,
            way_in,
            top,
            adder,
            bottom,
            count,
            jump,
            behind,
        } = self;
        let fam = REG_NAMES[0]
            .iter()
            .position(|name| name[1..] == **carry)
            .expect("the carry's register");
        let (low, byte) = (REG_NAMES[1][fam], REG_NAMES[3][fam]);
        // Eight limbs of `a` at (%rdi), which the states fill, eight of
        // `b` in the frame, which they fill with edges, the sum behind
        // `a`, and three trips.
        format!(
            "    movq %rdi, %rdx\n    leaq (%rsp), %r9\n    leaq 64(%rdi), %r10\n    movl $3, %r11d\n\
             {way_in}.LBB7:\n{top}    btl $0, {low}\n    {adder} (%r9), %rax\n    movq %rax, (%r10)\n    \
             movq 8(%rdx), %rax\n    {adder} 8(%r9), %rax\n    movl $0, {low}\n    setc {byte}\n\
             {bottom}{count}{jump}{behind}"
        )
    }

    /// [`Loop::limbs`] as the block layout leaves the main loop of a split
    /// limb loop where the order the pass appended its blocks in is the
    /// one emitted: behind the function's `ret`, entered by a `jmp` and
    /// left by one, and saying its frame again.
    fn behind_the_ret() -> Self {
        Loop {
            way_in: "    jmp .LBB7\n.LBB9:\n    movq %r8, %rcx\n    xorl %eax, %eax\n    ret\n",
            top: "    .cfi_def_cfa_offset 40\n    movq (%rdx), %rax\n",
            behind: "    jmp .LBB9",
            ..Loop::limbs()
        }
    }

    /// The loop keeps its carry in the flag, and computes what it did.
    fn is_kept(&self) -> String {
        kept_the_same(&self.text())
    }

    fn stays(&self, why: &str) {
        let body = self.text();
        let (changed, out) = kept(&body);
        assert!(!changed, "{why}: must stay\n{body}\n=>\n{out}");
    }
}

#[test]
fn the_model_runs_a_counted_loop() {
    let cpu = states().remove(0);
    let ran = cpu.clone().run_program(
        "    movl $5, %r11d\n    movl $0, %ecx\n.LBB1:\n    leaq 3(%rcx), %rcx\n    decq %r11\n    \
         jne .LBB1\n    ret\n    movl $99, %ecx",
    );
    assert_eq!((ran.regs[1], ran.regs[11]), (15, 0));
    // `dec` leaves CF, `sub` and `test` write it.
    for (count, carry) in [
        ("decq %r11", true),
        ("subq $1, %r11", false),
        ("testq %r11, %r11", false),
    ] {
        let ran = cpu.clone().run(&format!(
            "    movl $2, %r11d\n    movq $-1, %rcx\n    addq $1, %rcx\n    {count}"
        ));
        assert_eq!((ran.carry, ran.zero), (carry, false), "{count}");
    }
}

#[test]
fn a_carry_stays_in_the_flag_round_a_limb_loop() {
    let limbs = Loop::limbs();
    let out = limbs.is_kept();
    // The carry goes in once in front of the loop and comes out once
    // behind it; the count is a `dec` and nothing tests it again.
    assert_eq!(
        lines(&out)[4..],
        [
            "btl $0, %r8d",
            ".LBB7:",
            "movq (%rdx), %rax",
            "adcq (%r9), %rax",
            "movq %rax, (%r10)",
            "movq 8(%rdx), %rax",
            "adcq 8(%r9), %rax",
            "movq %rax, 8(%r10)",
            "leaq 16(%r10), %r10",
            "leaq 16(%r9), %r9",
            "leaq 16(%rdx), %rdx",
            "decq %r11",
            "jne .LBB7",
            "movl $0, %r8d",
            "setc %r8b",
            "movq %r8, %rcx",
            "xorl %eax, %eax",
            "ret",
        ],
        "{out}"
    );

    // Every spelling of the count, in both widths; a borrow; another
    // register; and nothing between the label and the `bt`.
    for count in [
        "    subq $1, %r11\n",
        "    addq $-1, %r11\n    testq %r11, %r11\n",
        "    decq %r11\n",
        "    decq %r11\n    testq %r11, %r11\n",
        "    subl $1, %r11d\n    testl %r11d, %r11d\n",
        "    decl %r11d\n",
    ] {
        let out = Loop {
            count,
            ..Loop::limbs()
        }
        .is_kept();
        let dec = if count.contains("r11d") {
            "decl %r11d"
        } else {
            "decq %r11"
        };
        let made = format!("{dec}\njne .LBB7\nmovl $0, %r8d\nsetc %r8b");
        assert!(out.contains(&made), "{out}");
        assert!(
            !out.contains("test") && !out.contains("sub") && !out.contains("add"),
            "{out}"
        );
    }
    Loop {
        adder: "sbbq",
        ..Loop::limbs()
    }
    .is_kept();
    let out = Loop {
        carry: "rbx",
        behind: "    movq %rbx, %rcx\n    xorl %eax, %eax\n    ret",
        ..Loop::limbs()
    }
    .is_kept();
    assert!(
        out.contains("btl $0, %ebx\n.LBB7:")
            && out.ends_with("setc %bl\nmovq %rbx, %rcx\nxorl %eax, %eax\nret"),
        "{out}"
    );
    let out = Loop {
        top: "    movq (%rdx), %rax\n    movq %rax, %rax\n",
        ..Loop::limbs()
    }
    .is_kept();
    assert!(
        out.contains("btl $0, %r8d\n.LBB7:\nmovq (%rdx), %rax\nmovq %rax, %rax\nadcq"),
        "{out}"
    );

    // Copies between the count, its test and the branch stay where they
    // are; one that writes the counter in front of its test keeps the test
    // and so the loop as it is.
    for (count, bottom) in [
        (
            "    subq $1, %r11\n    movq %rcx, %rbx\n    testq %r11, %r11\n",
            "decq %r11\nmovq %rcx, %rbx\njne .LBB7\nmovl $0, %r8d",
        ),
        (
            "    subq $1, %r11\n    movq %r11, %rbx\n    testq %r11, %r11\n    movq %rbx, %rcx\n",
            "decq %r11\nmovq %r11, %rbx\nmovq %rbx, %rcx\njne .LBB7\nmovl $0, %r8d",
        ),
        (
            "    subq $1, %r11\n    movq %r11, %rcx\n",
            "decq %r11\nmovq %r11, %rcx\njne .LBB7\nmovl $0, %r8d",
        ),
    ] {
        let out = Loop {
            count,
            ..Loop::limbs()
        }
        .is_kept();
        assert!(out.contains(bottom), "{out}");
    }
    Loop {
        count: "    subq $1, %r11\n    movq %rcx, %r11\n    testq %r11, %r11\n",
        ..Loop::limbs()
    }
    .stays("the counter is written in front of its test");

    // The carry out in another register than the carry in -- a phi and
    // what it receives round the loop -- and copied over on the way to the
    // branch: both are written once, behind the loop.
    let in_r13 = |text: String| {
        text.replace(
            "movl $0, %r8d\n    setc %r8b",
            "movl $0, %r13d\n    setc %r13b",
        )
    };
    for (bottom, count) in [
        (
            "    movq %rax, 8(%r10)\n    movq %r13, %r8\n    leaq 16(%r10), %r10\n    \
             leaq 16(%r9), %r9\n    leaq 16(%rdx), %rdx\n",
            Loop::limbs().count,
        ),
        (
            Loop::limbs().bottom,
            "    subq $1, %r11\n    movq %r13, %r8\n    testq %r11, %r11\n",
        ),
        (
            Loop::limbs().bottom,
            "    subq $1, %r11\n    testq %r11, %r11\n    movl %r13d, %r8d\n",
        ),
    ] {
        let body = in_r13(
            Loop {
                bottom,
                count,
                ..Loop::limbs()
            }
            .text(),
        );
        let out = kept_the_same(&body);
        assert!(
            out.contains("btl $0, %r8d\n.LBB7:")
                && out.contains("decq %r11\njne .LBB7\nmovl $0, %r13d\nsetc %r13b\nmov")
                && (out.contains("movq %r13, %r8\nmovq %r8, %rcx")
                    || out.contains("movl %r13d, %r8d\nmovq %r8, %rcx")),
            "{out}"
        );
    }

    // The count, then the steps, then its test: `do ... while (--n)` over
    // stepped pointers as it arrives when it is not split. The steps
    // become `lea` behind the `dec`.
    let out = Loop {
        bottom: "    movq %rax, 8(%r10)\n",
        count: "    subq $1, %r11\n    addq $16, %r10\n    addq $16, %r9\n    movq %r11, %rbx\n    \
                addq $16, %rdx\n    testq %r11, %r11\n",
        ..Loop::limbs()
    }
    .is_kept();
    assert!(
        out.contains(
            "movq %rax, 8(%r10)\ndecq %r11\nleaq 16(%r10), %r10\nleaq 16(%r9), %r9\n\
             movq %r11, %rbx\nleaq 16(%rdx), %rdx\njne .LBB7\nmovl $0, %r8d\nsetc %r8b"
        ),
        "{out}"
    );
    // Two steps of the counter: the test reads the second.
    let out = Loop {
        count: "    subq $1, %r11\n    addq $-1, %r11\n    testq %r11, %r11\n",
        way_in: "    movl $6, %r11d\n",
        ..Loop::limbs()
    }
    .is_kept();
    assert!(
        out.contains("leaq -1(%r11), %r11\ndecq %r11\njne .LBB7"),
        "{out}"
    );

    // An index or a pointer stepped by a constant is stepped by an `lea`,
    // which leaves the flag alone: up, down, in either width.
    let out = Loop {
        bottom: "    movq %rax, 8(%r10)\n    addq $16, %r10\n    subq $-16, %r9\n    addq $16, %rdx\n    \
                 addl $3, %ecx\n    subl $5, %ebx\n    subq $7, %r12\n",
        ..Loop::limbs()
    }
    .is_kept();
    assert!(
        out.contains(
            "movq %rax, 8(%r10)\nleaq 16(%r10), %r10\nleaq 16(%r9), %r9\nleaq 16(%rdx), %rdx\n\
             leal 3(%rcx), %ecx\nleal -5(%rbx), %ebx\nleaq -7(%r12), %r12\ndecq %r11\njne .LBB7"
        ),
        "{out}"
    );
    // ... but not a step between the adders, which is not on the way
    // round, and whose flags the next adder would have to survive.
    let body = Loop::limbs().text().replacen(
        "    movq %rax, (%r10)\n",
        "    movq %rax, (%r10)\n    addq $1, %rcx\n",
        1,
    );
    let out = kept_the_same(&body);
    assert!(out.contains("addq $1, %rcx"), "{out}");

    // The loop leaves through a jump up, to where the flags are written
    // before anything reads them (the old loop of a split one).
    let up = Loop::limbs().text().replacen(
        ".LBB7:",
        "    jmp .LBB6\n.LBB4:\n    movq %r8, %rcx\n    cmpq %rdx, %rdx\n    ret\n.LBB6:\n.LBB7:",
        1,
    );
    let up = up.replace(
        "    jne .LBB7\n    movq %r8, %rcx\n",
        "    jne .LBB7\n.LBB8:\n    movq %r9, %rbx\n    jmp .LBB4\n",
    );
    let out = kept_the_same(&up);
    assert!(
        out.contains("jne .LBB7\nmovl $0, %r8d\nsetc %r8b\n.LBB8:\nmovq %r9, %rbx\njmp .LBB4"),
        "{out}"
    );
    // ... but not to where they are read, or to a label that is not above.
    for (from, to) in [
        ("    cmpq %rdx, %rdx\n    ret", "    sete %cl\n    ret"),
        ("    cmpq %rdx, %rdx\n    ret", "    jb .LBB8\n    ret"),
        ("    jmp .LBB4\n", "    jmp elsewhere\n"),
        (
            "    movq %r9, %rbx\n    jmp .LBB4\n",
            "    pushq %rcx\n    jmp .LBB4\n",
        ),
        (
            "    movq %r9, %rbx\n    jmp .LBB4\n",
            "    jb .LBB4\n    jmp .LBB4\n",
        ),
    ] {
        assert!(up.contains(from), "{from}");
        let body = up.replace(from, to);
        let (changed, out) = kept(&body);
        assert!(!changed, "must stay:\n{body}\n=>\n{out}");
    }

    // The debugger's line marks are not lines of the loop: they stay where
    // they are, and the loop is kept round them.
    let marked = Loop::limbs()
        .text()
        .replacen(
            "    movq %rax, (%r10)\n",
            "    movq %rax, (%r10)\n    .loc 1 7 3\n",
            1,
        )
        .replacen(
            "    btl $0, %r8d\n",
            "    .loc 1 6 9\n    btl $0, %r8d\n",
            1,
        );
    let out = kept_the_same(&marked);
    assert!(
        out.contains(".loc 1 6 9\n")
            && out.contains(".loc 1 7 3\n")
            && out.contains("decq %r11\njne .LBB7\nmovl $0, %r8d\nsetc %r8b"),
        "{out}"
    );

    // A carry in that was just cleared is the flag the clearing left: no
    // `bt` at all. Cleared further up, or another register cleared, it is
    // tested.
    let out = Loop {
        way_in: "    xorl %r8d, %r8d\n",
        ..Loop::limbs()
    }
    .is_kept();
    assert!(
        !out.contains("bt") && out.contains("xorl %r8d, %r8d\n.LBB7:"),
        "{out}"
    );
    for way_in in [
        "    xorl %r8d, %r8d\n    movq %rdx, %rcx\n",
        "    xorl %ecx, %ecx\n",
        "    movl $1, %r8d\n",
        "    cmpq %rdx, %r9\n    je .LBB9\n",
        "    cmpq %rdx, %rdx\n    je .LBB9\n",
    ] {
        let out = Loop {
            way_in,
            behind: "    movq %r8, %rcx\n    xorl %eax, %eax\n.LBB9:\n    ret",
            ..Loop::limbs()
        }
        .is_kept();
        assert!(out.contains("btl $0, %r8d\n.LBB7:"), "{out}");
    }
}

/// The main loop of a split limb loop behind the function's `ret`: one
/// `jmp` enters it, and the carry goes into the flag in front of that.
#[test]
fn a_loop_entered_by_a_jump_takes_its_carry_in_front_of_the_jump() {
    let behind = Loop::behind_the_ret;
    let out = behind().is_kept();
    assert_eq!(
        lines(&out)[3..],
        [
            "movl $3, %r11d",
            "btl $0, %r8d",
            "jmp .LBB7",
            ".LBB9:",
            "movq %r8, %rcx",
            "xorl %eax, %eax",
            "ret",
            ".LBB7:",
            "movq (%rdx), %rax",
            "adcq (%r9), %rax",
            "movq %rax, (%r10)",
            "movq 8(%rdx), %rax",
            "adcq 8(%r9), %rax",
            "movq %rax, 8(%r10)",
            "leaq 16(%r10), %r10",
            "leaq 16(%r9), %r9",
            "leaq 16(%rdx), %rdx",
            "decq %r11",
            "jne .LBB7",
            "movl $0, %r8d",
            "setc %r8b",
            "jmp .LBB9",
        ],
        "{out}"
    );
    // The label keeps its place, and the block's frame is said in front of
    // every instruction of it, as it was.
    let (changed, whole) = kept_with_directives(&behind().text(), |_| {});
    assert!(changed);
    assert!(
        whole.contains("ret\n.LBB7:\n.cfi_def_cfa_offset 40\nmovq (%rdx), %rax\nadcq (%r9), %rax"),
        "{whole}"
    );
    assert_eq!(
        whole.matches(".cfi_def_cfa_offset 40").count(),
        1,
        "{whole}"
    );
    // Without a frame to say, with more than one directive for it, and
    // with a borrow, another register and a carry out in a second one.
    for top in [
        "    movq (%rdx), %rax\n",
        "    .cfi_restore_state\n    .cfi_def_cfa_offset 40\n    movq (%rdx), %rax\n",
    ] {
        let out = Loop { top, ..behind() }.is_kept();
        assert!(
            out.contains("movl $3, %r11d\nbtl $0, %r8d\njmp .LBB7\n"),
            "{out}"
        );
        assert!(
            out.contains("ret\n.LBB7:\nmovq (%rdx), %rax\nadcq (%r9), %rax"),
            "{out}"
        );
    }
    let out = Loop {
        adder: "sbbq",
        carry: "rbp",
        way_in: "    jmp .LBB7\n.LBB9:\n    movq %rbp, %rcx\n    xorl %eax, %eax\n    ret\n",
        ..behind()
    }
    .is_kept();
    assert!(out.contains("btl $0, %ebp\njmp .LBB7\n"), "{out}");
    assert!(
        out.contains("jne .LBB7\nmovl $0, %ebp\nsetc %bpl\njmp .LBB9"),
        "{out}"
    );
    let out = kept_the_same(
        &behind()
            .text()
            .replace(
                "movl $0, %r8d\n    setc %r8b",
                "movl $0, %r13d\n    setc %r13b",
            )
            .replace(
                "    testq %r11, %r11\n",
                "    movq %r13, %r8\n    testq %r11, %r11\n",
            ),
    );
    assert!(
        out.contains("jne .LBB7\nmovl $0, %r13d\nsetc %r13b\nmovq %r13, %r8\njmp .LBB9"),
        "{out}"
    );

    // A carry in cleared in front of the jump is the flag the clearing
    // left: no `bt`, and no line added.
    let cleared = Loop {
        way_in: "    xorl %r8d, %r8d\n    jmp .LBB7\n.LBB9:\n    movq %r8, %rcx\n    xorl %eax, %eax\n    ret\n",
        ..behind()
    };
    let out = cleared.is_kept();
    assert!(!out.contains("bt"), "{out}");
    assert!(out.contains("xorl %r8d, %r8d\njmp .LBB7\n"), "{out}");
    assert_eq!(lines(&out).len(), lines(&cleared.text()).len() - 3, "{out}");
    // Cleared further up, another register cleared, or cleared on one of
    // two ways to the jump: it is tested, in front of the jump and behind
    // the label the other way arrives at.
    for (way_in, made) in [
        (
            "    xorl %r8d, %r8d\n    movq %rdx, %rcx\n    jmp .LBB7\n",
            "movq %rdx, %rcx\nbtl $0, %r8d\njmp .LBB7\n",
        ),
        (
            "    xorl %ecx, %ecx\n    jmp .LBB7\n",
            "xorl %ecx, %ecx\nbtl $0, %r8d\njmp .LBB7\n",
        ),
        (
            "    testq %rsi, %rsi\n    je .LBB5\n    xorl %r8d, %r8d\n.LBB5:\n    jmp .LBB7\n",
            "xorl %r8d, %r8d\n.LBB5:\nbtl $0, %r8d\njmp .LBB7\n",
        ),
        (
            "    testq %rsi, %rsi\n    je .LBB9\n    jmp .LBB7\n",
            "je .LBB9\nbtl $0, %r8d\njmp .LBB7\n",
        ),
    ] {
        let way_in = format!("{way_in}.LBB9:\n    movq %r8, %rcx\n    xorl %eax, %eax\n    ret\n");
        let out = Loop {
            way_in: &way_in,
            ..behind()
        }
        .is_kept();
        assert!(out.contains(made), "{out}");
        assert_eq!(out.matches("btl").count(), 1, "{out}");
    }

    // What stands above the label is a jump through a register: no way in
    // either, the label's address is taken nowhere.
    let out = Loop {
        way_in: "    jmp .LBB7\n.LBB9:\n    movq %r8, %rcx\n    xorl %eax, %eax\n    ret\n    jmp *%rcx\n",
        ..behind()
    }
    .is_kept();
    assert!(
        out.contains("movl $3, %r11d\nbtl $0, %r8d\njmp .LBB7\n"),
        "{out}"
    );

    // The jump stands directly above the label, or below the loop: the
    // line that is added moves what stands behind it, and nothing else.
    let out = Loop {
        way_in: "    jmp .LBB7\n",
        ..Loop::limbs()
    }
    .is_kept();
    assert!(
        out.contains("movl $3, %r11d\nbtl $0, %r8d\njmp .LBB7\n.LBB7:\nmovq (%rdx), %rax\nadcq"),
        "{out}"
    );
    let below = Loop {
        way_in: "    jmp .LBB1\n.LBB9:\n    movq %r8, %rcx\n    xorl %eax, %eax\n    ret\n",
        behind: "    jmp .LBB9\n.LBB1:\n    jmp .LBB7",
        ..behind()
    };
    let out = below.is_kept();
    assert!(
        out.ends_with(
            "jne .LBB7\nmovl $0, %r8d\nsetc %r8b\njmp .LBB9\n.LBB1:\nbtl $0, %r8d\njmp .LBB7"
        ),
        "{out}"
    );
    assert!(out.contains("movl $3, %r11d\njmp .LBB1\n.LBB9:"), "{out}");

    // Two loops in one file, each with a line added above it: the second
    // is found where the first one's line moved it to, and each is made
    // what it is made alone. Then the same with the jumps below the loops.
    for first in [behind(), below] {
        let renamed = |text: &str| {
            text.replace(".LBB7", ".LBB17")
                .replace(".LBB9", ".LBB19")
                .replace(".LBB1\n", ".LBB11\n")
                .replace(".LBB1:", ".LBB11:")
                .replace("%r8d", "%ebp")
                .replace("%r8b", "%bpl")
                .replace("%r8", "%rbp")
        };
        let first = first.text();
        let second = renamed(&first);
        let (changed, both) = kept(&format!("{first}\n{second}"));
        assert!(changed);
        assert_eq!(
            both,
            format!("{}\n{}", kept_the_same(&first), kept_the_same(&second)),
            "{both}"
        );
        assert_eq!(both.matches("bt").count(), 2, "{both}");
    }

    // Far from its jump in, and from where its jump out goes, but within
    // reach of both.
    let filler = "    movq %rcx, %rsi\n".repeat(500);
    let far = format!(
        "    jmp .LBB7\n.LBB9:\n    movq %r8, %rcx\n    xorl %eax, %eax\n    ret\n{filler}    ret\n"
    );
    let out = Loop {
        way_in: &far,
        ..behind()
    }
    .is_kept();
    assert!(
        out.contains("movl $3, %r11d\nbtl $0, %r8d\njmp .LBB7\n"),
        "{out}"
    );
}

#[test]
fn a_loop_whose_carry_cannot_stay_in_the_flag_is_left_alone() {
    let limbs = Loop::limbs;
    // A flag written on the way round: between the label and the `bt`,
    // between the carry out and the count.
    Loop {
        top: "    movq (%rdx), %rax\n    addq $0, %rcx\n",
        ..limbs()
    }
    .stays("a flag written in front of the carry in");
    Loop {
        bottom: "    movq %rax, 8(%r10)\n    addq %rcx, %r10\n    leaq 16(%r9), %r9\n    leaq 16(%rdx), %rdx\n",
        ..limbs()
    }
    .stays("a pointer stepped by a register");
    Loop {
        bottom: "    movq %rax, 8(%r10)\n    leaq 16(%r10), %r10\n    leaq 16(%r9), %r9\n    \
                 leaq 16(%rdx), %rdx\n    addq $16, 8(%rsp)\n",
        ..limbs()
    }
    .stays("a step in memory");
    Loop {
        bottom: "    movq %rax, 8(%r10)\n    leaq 16(%r10), %r10\n    leaq 16(%r9), %r9\n    \
                 leaq 16(%rdx), %rdx\n    shlq $1, %rcx\n",
        ..limbs()
    }
    .stays("a shift in front of the count");
    Loop {
        bottom: "    movq %rax, 8(%r10)\n    leaq 16(%r10), %r10\n    leaq 16(%r9), %r9\n    \
                 leaq 16(%rdx), %rdx\n    cmpq %r9, %rdx\n",
        ..limbs()
    }
    .stays("a compare in front of the count");
    // A directive among the lines, which would trade places with them:
    // unwind state, alignment, data, another section.
    for directive in [
        ".cfi_remember_state",
        ".p2align 4",
        ".byte 0x90",
        ".section .rodata",
    ] {
        let body = limbs().text().replacen(
            "    movq %rax, (%r10)\n",
            &format!("    movq %rax, (%r10)\n    {directive}\n"),
            1,
        );
        let (changed, out) = kept(&body);
        assert!(!changed, "{directive} must stay:\n{out}");
    }
    // The carry out in another register than the carry in: with no copy
    // into the carry's, with the copy the other way, in front of the
    // `setc`, or not the only reader of that register.
    let in_r13 = |text: String| {
        text.replace(
            "movl $0, %r8d\n    setc %r8b",
            "movl $0, %r13d\n    setc %r13b",
        )
    };
    let with_bottom = |extra: &str| {
        let bottom = format!("    movq %rax, 8(%r10)\n{extra}    leaq 16(%r10), %r10\n");
        in_r13(
            Loop {
                bottom: &bottom,
                ..limbs()
            }
            .text(),
        )
    };
    for body in [
        with_bottom(""),
        with_bottom("    movq %r8, %r13\n"),
        with_bottom("    movq %r13, %r8\n    movq %r13, %rcx\n"),
        with_bottom("    movq %r13, %r8\n    movq %r8, %rcx\n"),
        with_bottom("    movq %r13, %r8\n    movq %r13, %r8\n"),
        limbs().text().replace(
            "    movl $0, %r8d\n    setc %r8b",
            "    movq %r13, %r8\n    movl $0, %r13d\n    setc %r13b",
        ),
    ] {
        let (changed, out) = kept(&body);
        assert!(!changed, "must stay:\n{body}\n=>\n{out}");
    }
    // Between the count and its test: the counter stepped by another
    // constant, a step by a register, a reader of the flags. And a step
    // behind a count that has no test: the branch reads the step's flags.
    for count in [
        "    subq $1, %r11\n    addq $16, %r11\n    testq %r11, %r11\n",
        "    subq $1, %r11\n    addq %rcx, %r10\n    testq %r11, %r11\n",
        "    subq $1, %r11\n    setc %cl\n    testq %r11, %r11\n",
        "    subq $1, %r11\n    addq $16, %r10\n",
    ] {
        Loop { count, ..limbs() }.stays("the count is not what the branch reads");
    }
    // The carry's register read or written by the loop.
    for (top, bottom) in [
        (
            "    movq (%rdx), %rax\n    movq %r8, %rcx\n",
            limbs().bottom,
        ),
        ("    movq (%rdx,%r8,8), %rax\n", limbs().bottom),
        ("    movq (%rdx), %rax\n    movl $1, %r8d\n", limbs().bottom),
        (
            limbs().top,
            "    movq %rax, 8(%r10)\n    movq %r8, 24(%r10)\n    leaq 16(%r10), %r10\n    \
             leaq 16(%r9), %r9\n    leaq 16(%rdx), %rdx\n",
        ),
        (
            limbs().top,
            "    movq %rax, 8(%r10)\n    leaq 16(%r10,%r8), %r10\n    leaq 16(%r9), %r9\n    \
             leaq 16(%rdx), %rdx\n",
        ),
    ] {
        Loop {
            top,
            bottom,
            ..limbs()
        }
        .stays("the carry's register named in the loop");
    }
    Loop {
        carry: "r11",
        ..limbs()
    }
    .stays("the carry's register is the count");
    // The count: by two, of another register than the one tested, in
    // another width than the test, compared instead of tested.
    for count in [
        "    subq $2, %r11\n",
        "    subq $1, %r11\n    testq %rcx, %rcx\n",
        "    subq $1, %r11\n    testl %r11d, %r11d\n",
        "    subl $1, %r11d\n    testq %r11, %r11\n",
        "    subq $1, %r11\n    cmpq $0, %r11\n",
        "    subq $1, %r11\n    testq %r11, %rcx\n",
        "    leaq -1(%r11), %r11\n    testq %r11, %r11\n",
    ] {
        Loop { count, ..limbs() }.stays("a count that is no `sub $1; test`");
    }
    // Another branch closes the loop, or the `jne` goes elsewhere.
    for jump in [
        "    je .LBB7\n",
        "    jb .LBB7\n",
        "    jne .LBB8\n.LBB8:\n",
        "    jmp .LBB7\n",
    ] {
        Loop { jump, ..limbs() }.stays("a loop not closed by its own `jne`");
    }
    // The label on top is named once, and not by the branch at the bottom:
    // no loop.
    Loop {
        way_in: "    jmp .LBB7\n.LBB6:\n",
        jump: "    jne .LBB8\n.LBB8:\n",
        ..limbs()
    }
    .stays("the branch at the bottom goes elsewhere");
    // A second way in: the label named twice, or reached by a jump alone.
    Loop {
        behind: "    movq %r8, %rcx\n    xorl %eax, %eax\n    testq %rcx, %rcx\n    jne .LBB7\n    ret",
        ..limbs()
    }
    .stays("two branches to the label");
    Loop {
        way_in: "    .p2align 4\n",
        ..limbs()
    }
    .stays("an aligned label");
    // A label that the file defines twice names no one place, whichever
    // way the loop is entered.
    Loop {
        behind: "    movq %r8, %rcx\n    xorl %eax, %eax\n    ret\n.LBB7:\n    ret",
        ..limbs()
    }
    .stays("a label defined twice");
    Loop {
        behind: "    jmp .LBB9\n.LBB7:\n    ret",
        ..Loop::behind_the_ret()
    }
    .stays("a label defined twice, entered by a jump");
    // Entered by a jump, and by something else as well: a second jump, a
    // conditional one, the line above (which falls in, or is a call that
    // may come back), a taken address; or by nothing that can be told.
    let behind = Loop::behind_the_ret;
    let tail = ".LBB9:\n    movq %r8, %rcx\n    xorl %eax, %eax\n    ret\n";
    for (way_in, why) in [
        (
            "    testq %rsi, %rsi\n    je .LBB5\n    jmp .LBB7\n.LBB5:\n    jmp .LBB7\n",
            "two jumps into the loop",
        ),
        (
            "    testq %rsi, %rsi\n    je .LBB7\n    jmp .LBB7\n",
            "a jump and a branch into the loop",
        ),
        (
            "    testq %rsi, %rsi\n    jne .LBB7\n",
            "a branch into the loop and no jump",
        ),
        (
            "    leaq .LBB7(%rip), %rcx\n    jmp .LBB7\n",
            "the label's address taken",
        ),
        (
            "    leaq .LBB7(%rip), %rcx\n    jmp *%rcx\n",
            "entered through its address",
        ),
        ("    jmp *%rcx\n", "entered by nothing that names it"),
        ("", "entered by nothing at all"),
    ] {
        let way_in = format!("{way_in}{tail}");
        Loop {
            way_in: &way_in,
            ..behind()
        }
        .stays(why);
    }
    for (above, why) in [
        (
            "    movq %r8, %rcx\n",
            "a jump in, and the line above falls in",
        ),
        ("    call abort\n", "a jump in, and a call above"),
        (
            "    ret\n    .p2align 4\n",
            "a jump in, and an aligned label",
        ),
        (
            "    ret\n    .byte 0x90\n",
            "a jump in, and data above the label",
        ),
    ] {
        let way_in = format!("    jmp .LBB7\n.LBB9:\n    movq %r8, %rcx\n{above}");
        Loop {
            way_in: &way_in,
            ..behind()
        }
        .stays(why);
    }
    // The jump in is too far above the loop to be looked for; so is the
    // label of the jump out, each alone. Half as far, either is found.
    for (lines_between, found) in [(500, true), (1100, false)] {
        let filler = "    movq %rcx, %rsi\n".repeat(lines_between);
        let jump_in = format!("    jmp .LBB7\n{filler}    ret\n");
        let jump_out = format!("    jmp .LBB1\n{tail}{filler}.LBB1:\n");
        for (way_in, top, behind) in [
            (
                &jump_in,
                "    .cfi_def_cfa_offset 40\n    movq (%rdx), %rax\n",
                "    movq %r8, %rcx\n    xorl %eax, %eax\n    ret",
            ),
            (&jump_out, "    movq (%rdx), %rax\n", "    jmp .LBB9"),
        ] {
            let far = Loop {
                way_in,
                top,
                behind,
                ..limbs()
            };
            if found {
                far.is_kept();
            } else {
                far.stays("a jump out of reach");
            }
        }
    }
    // The jump is inline assembly, or a line that was deleted.
    let body = behind().text();
    let jump = 2 + lines(&body)
        .iter()
        .position(|line| *line == "jmp .LBB7")
        .expect("the jump");
    let (changed, out) = kept_unless(&body, |infos| infos[jump].kind = LineKind::InlineAsm);
    assert!(!changed, "{out}");
    let (changed, out) = kept_unless(&body, |infos| infos[jump].kind = LineKind::Nop);
    assert!(!changed, "{out}");
    // The frame said behind the label of a loop that the line above falls
    // into: the `bt` would stand above the label and the directive with it.
    Loop {
        top: "    .cfi_def_cfa_offset 40\n    movq (%rdx), %rax\n",
        ..limbs()
    }
    .stays("unwind directives behind a label that moves");
    // Entered by a jump, with a directive that is not the block's frame
    // behind the label, or one further down.
    for top in [
        "    .p2align 4\n    movq (%rdx), %rax\n",
        "    .byte 0x90\n    movq (%rdx), %rax\n",
        "    .cfi_def_cfa_offset 40\n    .p2align 4\n    movq (%rdx), %rax\n",
        "    movq (%rdx), %rax\n    .cfi_def_cfa_offset 40\n",
        "    .cfi_def_cfa_offset 40\n    movq (%rdx), %rax\n    .cfi_def_cfa_offset 48\n",
    ] {
        Loop { top, ..behind() }.stays("a directive in a loop entered by a jump");
    }
    // Not one straight run: a label, a call, a push inside.
    for top in [
        "    movq (%rdx), %rax\n.LBB8:\n",
        "    call g\n    movq (%rdx), %rax\n",
        "    pushq %rcx\n    popq %rcx\n    movq (%rdx), %rax\n",
    ] {
        Loop { top, ..limbs() }.stays("more than one block");
    }
    // The flags of the count are read behind the loop.
    for behind in [
        "    adcq $0, %rcx\n    xorl %eax, %eax\n    ret",
        "    movq %r8, %rcx\n    sete %cl\n    xorl %eax, %eax\n    ret",
        "    jb .LBB9\n    movq %r8, %rcx\n.LBB9:\n    xorl %eax, %eax\n    ret",
    ] {
        Loop { behind, ..limbs() }.stays("flags read behind the loop");
    }
    // A line of the loop that may not be rewritten: every one of them is
    // given another text. (The first six lines are `foo:`, the unwind
    // directive and what comes before the label.)
    let body = Loop::limbs().text();
    let loop_lines = 6..6 + 16;
    assert!(kept_unless(&body, |infos| infos[5].pinned = true).0);
    assert!(kept_unless(&body, |infos| infos[22].pinned = true).0);
    for line in loop_lines {
        let (pinned, _) = kept_unless(&body, |infos| infos[line].pinned = true);
        let (volatile, _) = kept_unless(&body, |infos| infos[line].volatile_access = true);
        assert!(!pinned && !volatile, "line {line} of\n{body}");
    }

    // The first to read the flag behind the `bt` is no adder; the carry
    // out is not the carry's (another byte, no zero, a `setnc`).
    for body in [
        Loop::limbs()
            .text()
            .replacen("adcq (%r9), %rax", "setc %cl\n    adcq (%r9), %rax", 1),
        Loop::limbs()
            .text()
            .replacen("adcq (%r9), %rax", "rclq $1, %rax", 1),
        Loop::limbs().text().replace("setc %r8b", "setc %cl"),
        Loop::limbs().text().replace("setc %r8b", "setnc %r8b"),
        Loop::limbs()
            .text()
            .replace("movl $0, %r8d", "movl $0, %ecx"),
        Loop::limbs()
            .text()
            .replace("movl $0, %r8d", "movl $1, %r8d"),
        Loop::limbs().text().replace("    movl $0, %r8d\n", ""),
        Loop::limbs().text().replace("btl $0, %r8d", "btl $1, %r8d"),
        Loop::limbs()
            .text()
            .replace("btl $0, %r8d", "btl $0, 8(%rsp)"),
        Loop::limbs().text().replace("    btl $0, %r8d\n", ""),
    ] {
        let (changed, out) = kept(&body);
        assert!(!changed, "must stay:\n{body}\n=>\n{out}");
    }
}

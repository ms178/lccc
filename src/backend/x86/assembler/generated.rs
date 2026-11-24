//! The cheaper statements of a generated text.
//!
//! A text is assembled as written. So that what the code generator writes
//! is the cheaper instruction all the same -- `movl $1, %eax` for `movq
//! $1, %rax`, `movq %rsp, %rsi` for `leaq (%rsp), %rsi`, the replacements
//! of [`encoder::optimize`](super::encoder::optimize) -- it is replaced in
//! the text, once, after every emitter has run and before anything reads
//! the result: `-S` shows it, GNU as and Clang assemble it to the bytes
//! lccc does, and the debug information that measures addresses in the
//! text measures the ones the object has.
//!
//! [`rewrite`] reads every line and parses few: only a statement whose
//! mnemonic and operands can have a replacement at all
//! ([`optimize::candidate`]). What the encoder then chooses is written
//! back as text and kept only if that text, parsed and encoded as
//! written, is byte for byte and relocation for relocation what the
//! encoder chose; a statement this module cannot write that way stays as
//! it was.
//!
//! Not touched:
//!
//! * the bodies of inline assembly, which the code generator brackets
//!   with `#APP` and `#NO_APP`: a person wrote those, and may have counted
//!   their bytes;
//! * the statement behind data or behind a prefix on a line of its own,
//!   which may be the rest of an instruction somebody put together by
//!   hand (GNU as has the same rule);
//! * everything behind a directive that changes what a statement means:
//!   another code size, another syntax, an included file, and any
//!   statement by the name of a macro.

use super::encoder::optimize::{self, Atomic};
use super::parser::{
    Displacement, ImmediateValue, Instruction, MemoryOperand, Operand, Optimize, Register,
    is_prefix_word, parse_statement,
};
use std::fmt::Write as _;

/// The encoder of one mode, as this module uses it.
pub(crate) trait Coder: Atomic {
    /// Encode `instr`, returning the instruction that replaced it if one
    /// did.
    fn choose(&mut self, instr: &Instruction) -> Result<Option<Instruction>, String>;
    /// What has been written: the bytes, and each relocation.
    fn encoding(&self) -> (Vec<u8>, Vec<Relocated>);
}

/// One relocation: offset, symbol, type, addend, subtracted symbol.
pub(crate) type Relocated = (u64, String, u32, i64, Option<String>);

impl Coder for super::encoder::InstructionEncoder {
    fn choose(&mut self, instr: &Instruction) -> Result<Option<Instruction>, String> {
        self.encode_choosing(instr)
    }

    fn encoding(&self) -> (Vec<u8>, Vec<Relocated>) {
        let relocations = self
            .relocations
            .iter()
            .map(|r| {
                (
                    r.offset,
                    r.symbol.clone(),
                    r.reloc_type,
                    r.addend,
                    r.diff_symbol.clone(),
                )
            })
            .collect();
        (self.bytes.clone(), relocations)
    }
}

/// `text`, generated for 64-bit code, with every statement the compiler
/// wrote replaced by its cheaper instruction.
pub(crate) fn rewrite(text: String, optimize: Optimize) -> String {
    rewrite_with(
        text,
        optimize,
        true,
        super::encoder::InstructionEncoder::new,
    )
}

/// The directives that put nothing in front of the next statement.
const SILENT: [&str; 22] = [
    ".loc",
    ".cfi_",
    ".p2align",
    ".align",
    ".balign",
    ".type",
    ".size",
    ".globl",
    ".global",
    ".section",
    ".text",
    ".file",
    ".weak",
    ".hidden",
    ".local",
    ".set",
    ".ident",
    ".pushsection",
    ".popsection",
    ".previous",
    ".protected",
    ".internal",
];

/// What the lines so far have said about the ones to come.
struct Reader {
    /// 64-bit code, as generated.
    long: bool,
    /// Inside `#APP` ... `#NO_APP`: the body of an inline assembly.
    user: bool,
    /// Behind `.intel_syntax`.
    intel: bool,
    /// Behind a `.code` directive of another size than the generated one.
    resized: bool,
    /// Behind a directive this module does not follow: `.include`,
    /// `.noopt`.
    lost: bool,
    /// Behind data, a prefix on a line of its own, or the end of an inline
    /// assembly: the next statement may be the rest of something.
    bare: bool,
    /// The macros defined so far: a statement by one of these names is
    /// that macro.
    macros: Vec<String>,
}

impl Reader {
    /// Take note of `body`, a line without its indentation, and say
    /// whether it is a statement of the compiler's that may be replaced.
    fn read(&mut self, body: &str) -> bool {
        let Some(&first) = body.as_bytes().first() else {
            return false;
        };
        if first == b'#' {
            // The two comments that say whose text this is.
            if body.starts_with("#APP") {
                self.user = true;
            } else if body.starts_with("#NO_APP") {
                self.user = false;
                self.bare = true;
            }
            return false;
        }
        if self.user {
            self.directives(body);
            return false;
        }
        let word = body.split([' ', '\t']).next().unwrap_or(body);
        if word.ends_with(':') {
            // A label.
            return false;
        }
        if first == b'.' {
            self.directives(body);
            if !SILENT.iter().any(|silent| word.starts_with(silent)) {
                self.bare = true;
            }
            return false;
        }
        // `{nf} addq $1, %rsi, %rax` is a statement like another.
        if !first.is_ascii_lowercase() && !body.starts_with("{nf} ") {
            self.bare = true;
            return false;
        }
        let bare = std::mem::replace(&mut self.bare, false);
        if word.len() == body.trim_end().len() && is_prefix_word(word) {
            // A prefix on a line of its own belongs to the next statement.
            self.bare = true;
            return false;
        }
        !bare && !self.intel && !self.resized && !self.lost
    }

    /// Take note of the directives of a line that change what the
    /// statements behind it mean. A person's line can have one anywhere.
    fn directives(&mut self, line: &str) {
        let mut rest = line;
        while let Some(at) = rest.find('.') {
            rest = &rest[at + 1..];
            if let Some(size) = rest.strip_prefix("code") {
                let size = size
                    .split(|c: char| !c.is_ascii_alphanumeric())
                    .next()
                    .unwrap_or("");
                self.resized = size != if self.long { "64" } else { "32" };
            } else if rest.starts_with("intel_syntax") {
                self.intel = true;
            } else if rest.starts_with("att_syntax") {
                // Registers without their `%` are still not this syntax.
                self.intel = line.contains("noprefix");
            } else if rest.starts_with("include") || rest.starts_with("noopt") {
                self.lost = true;
            } else if let Some(name) = rest.strip_prefix("macro") {
                let name = name
                    .trim_start()
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .next()
                    .unwrap_or("");
                if !name.is_empty() {
                    self.macros.push(name.to_string());
                }
            }
        }
    }
}

/// [`rewrite`] for the encoder `coder` makes, of 64-bit code (`long`) or
/// of 32-bit code.
pub(crate) fn rewrite_with<C: Coder>(
    text: String,
    optimize: Optimize,
    long: bool,
    coder: impl Fn() -> C,
) -> String {
    if optimize == Optimize::Never {
        return text;
    }
    let mut reader = Reader {
        long,
        user: false,
        intel: false,
        resized: false,
        lost: false,
        bare: false,
        macros: Vec::new(),
    };
    // The rewritten text, begun at the first replacement: `text[..copied]`
    // is in it.
    let mut out: Option<String> = None;
    let mut copied = 0;
    let mut encoder: Option<C> = None;
    let mut start = 0;
    while start < text.len() {
        let end = text[start..].find('\n').map_or(text.len(), |at| start + at);
        let line = &text[start..end];
        let body = line.trim_start();
        if reader.read(body) {
            let encoder = encoder.get_or_insert_with(&coder);
            if let Some(statement) = replacement(body, optimize, long, &reader.macros, encoder) {
                let out = out.get_or_insert_with(|| String::with_capacity(text.len()));
                out.push_str(&text[copied..start]);
                out.push_str(&line[..line.len() - body.len()]);
                out.push_str(&statement);
                copied = end;
            }
        }
        start = end + 1;
    }
    match out {
        Some(mut out) => {
            out.push_str(&text[copied..]);
            out
        }
        None => text,
    }
}

/// The statement to write in place of `body`, a line without its
/// indentation, if the encoder has a cheaper instruction for it.
fn replacement<C: Coder>(
    body: &str,
    optimize: Optimize,
    long: bool,
    macros: &[String],
    encoder: &mut C,
) -> Option<String> {
    let body = body.trim_end();
    let (flagless, statement) = match body.strip_prefix("{nf} ") {
        Some(statement) => (true, statement.trim_start()),
        None => (false, body),
    };
    let (mnemonic, operands) = match statement.find([' ', '\t']) {
        Some(at) => (&statement[..at], statement[at..].trim_start()),
        None => (statement, ""),
    };
    let candidate = if flagless {
        optimize::flagless_candidate(mnemonic)
    } else {
        optimize::candidate(mnemonic, operands, long, optimize)
    };
    if !candidate {
        return None;
    }
    // One statement and nothing else: no comment, no second statement, no
    // string.
    let is_macro = |name: &str| macros.iter().any(|defined| defined == name);
    if body
        .bytes()
        .any(|byte| matches!(byte, b'#' | b';' | b'"' | b'/'))
        || is_macro(mnemonic)
    {
        return None;
    }
    let written = parse_statement(body, optimize)?;
    if optimize::cheaper(&written, long).is_empty() {
        return None;
    }
    let none = (0, 0, 0);
    encoder.rewind(none);
    let chosen = encoder.choose(&written).ok()??;
    let wanted = encoder.encoding();
    // The text of it, which has to assemble as written to what was chosen.
    let text = render(&chosen)?;
    if is_macro(&chosen.mnemonic) {
        return None;
    }
    let read = parse_statement(&text, Optimize::Never)?;
    encoder.rewind(none);
    encoder.written(&read).ok()?;
    (encoder.encoding() == wanted).then_some(text)
}

/// An instruction as AT&T text, if it is one this module writes: a
/// mnemonic on plain registers, numbers, symbols and undecorated memory
/// operands.
fn render(instr: &Instruction) -> Option<String> {
    if !instr.prefixes.is_empty()
        || instr.force_evex
        || instr.force_rex2
        || instr.vex_hint.is_some()
        || instr.dfv != 0
    {
        return None;
    }
    let mut text = String::with_capacity(40);
    if instr.nf {
        text.push_str("{nf} ");
    }
    text.push_str(&instr.mnemonic);
    for (position, operand) in instr.operands.iter().enumerate() {
        text.push_str(if position == 0 { " " } else { ", " });
        match operand {
            Operand::Register(register) => render_register(&mut text, register)?,
            Operand::Immediate(ImmediateValue::Integer(value)) => {
                write!(text, "${value}").ok()?;
            }
            Operand::Immediate(ImmediateValue::Symbol(name)) => {
                text.push('$');
                text.push_str(name);
            }
            Operand::Immediate(ImmediateValue::SymbolPlusOffset(name, offset)) => {
                write!(text, "${name}{offset:+}").ok()?;
            }
            Operand::Memory(address) => render_address(&mut text, address)?,
            _ => return None,
        }
    }
    Some(text)
}

fn render_register(text: &mut String, register: &Register) -> Option<()> {
    if register.mask.is_some() || register.zeroing || register.broadcast.is_some() {
        return None;
    }
    text.push('%');
    text.push_str(&register.name);
    Some(())
}

fn render_address(text: &mut String, address: &MemoryOperand) -> Option<()> {
    if address.mask.is_some() || address.zeroing || address.broadcast.is_some() {
        return None;
    }
    if let Some(segment) = &address.segment {
        text.push('%');
        text.push_str(segment);
        text.push(':');
    }
    let registers = address.base.is_some() || address.index.is_some();
    match &address.displacement {
        Displacement::None if registers => {}
        Displacement::Integer(value) => write!(text, "{value}").ok()?,
        Displacement::Symbol(name) => text.push_str(name),
        Displacement::SymbolPlusOffset(name, offset) | Displacement::SymbolAddend(name, offset) => {
            write!(text, "{name}{offset:+}").ok()?;
        }
        Displacement::SymbolMod(name, modifier) => write!(text, "{name}@{modifier}").ok()?,
        Displacement::SymbolModAddend(name, modifier, offset) => {
            write!(text, "{name}@{modifier}{offset:+}").ok()?;
        }
        _ => return None,
    }
    if registers {
        text.push('(');
        if let Some(base) = &address.base {
            render_register(text, base)?;
        }
        if let Some(index) = &address.index {
            text.push(',');
            render_register(text, index)?;
            if let Some(scale) = address.scale {
                write!(text, ",{scale}").ok()?;
            }
        }
        text.push(')');
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::x86::assembler::encoder::InstructionEncoder;

    fn bytes(statement: &str) -> Vec<u8> {
        let instr = parse_statement(statement, Optimize::Never)
            .unwrap_or_else(|| panic!("`{statement}` is not a statement"));
        let mut encoder = InstructionEncoder::new();
        encoder
            .encode(&instr)
            .unwrap_or_else(|error| panic!("`{statement}`: {error}"));
        encoder.bytes
    }

    #[test]
    fn the_compilers_statements_are_replaced_in_the_text() {
        let text = "\t.text\n\
                    \t.globl f\n\
                    f:\n\
                    \tpushq %rbp\n\
                    \tmovq $1, %rax\n\
                    \tleaq (%rsp), %rsi\n\
                    .L1:\n\
                    \tmovsd %xmm0, (%rdi)\n\
                    \tmovq $-1, %rcx\n\
                    \tret\n";
        let wanted = "\t.text\n\
                      \t.globl f\n\
                      f:\n\
                      \tpushq %rbp\n\
                      \tmovl $1, %eax\n\
                      \tmovq %rsp, %rsi\n\
                      .L1:\n\
                      \tmovlps %xmm0, (%rdi)\n\
                      \tmovq $-1, %rcx\n\
                      \tret\n";
        assert_eq!(rewrite(text.to_string(), Optimize::Faster), wanted);
        assert_eq!(rewrite(text.to_string(), Optimize::Never), text);
    }

    #[test]
    fn a_text_without_a_replacement_is_returned_as_it_is() {
        let text = "f:\n\tmovl $1, %eax\n\tret\n".to_string();
        let at = text.as_ptr();
        let same = rewrite(text, Optimize::Smaller);
        assert_eq!(same.as_ptr(), at, "copied a text nothing was replaced in");
    }

    #[test]
    fn what_a_person_wrote_is_left_alone() {
        // Inline assembly; the statement behind its end, behind data and
        // behind a lone prefix; then the compiler's again.
        let text = "\tmovq $1, %rax\n\
                    #APP\n\
                    \tmovq $1, %rcx\n\
                    #NO_APP\n\
                    \tmovq $1, %rdx\n\
                    \tmovq $1, %rbx\n\
                    \t.byte 0x48\n\
                    \tmovq $1, %rsi\n\
                    \tlock\n\
                    \tmovq $1, %rdi\n\
                    \tmovq $1, %r8 # a comment\n\
                    \tmovq $1, %r9\n";
        let wanted = "\tmovl $1, %eax\n\
                      #APP\n\
                      \tmovq $1, %rcx\n\
                      #NO_APP\n\
                      \tmovq $1, %rdx\n\
                      \tmovl $1, %ebx\n\
                      \t.byte 0x48\n\
                      \tmovq $1, %rsi\n\
                      \tlock\n\
                      \tmovq $1, %rdi\n\
                      \tmovq $1, %r8 # a comment\n\
                      \tmovl $1, %r9d\n";
        assert_eq!(rewrite(text.to_string(), Optimize::Faster), wanted);
    }

    #[test]
    fn a_directive_that_changes_the_language_ends_it() {
        for directive in [
            ".intel_syntax noprefix",
            ".code32",
            ".code16",
            ".include \"other.s\"",
            ".noopt",
        ] {
            let text = format!("\tmovq $1, %rax\n\t{directive}\n\tmovq $1, %rcx\n");
            let wanted = format!("\tmovl $1, %eax\n\t{directive}\n\tmovq $1, %rcx\n");
            assert_eq!(rewrite(text, Optimize::Faster), wanted, "{directive}");
        }
        // A directive inside an inline assembly counts too, and so does a
        // macro by the name of an instruction.
        let text = "#APP\n\t.code32\n#NO_APP\n\tnop\n\tmovq $1, %rax\n";
        assert_eq!(rewrite(text.to_string(), Optimize::Faster), text);
        let text = "#APP\n.macro movq a, b\n.endm\n#NO_APP\n\tnop\n\tmovq $1, %rax\n";
        assert_eq!(rewrite(text.to_string(), Optimize::Faster), text);
        // Back in the generated size, it goes on.
        let text = "#APP\n\t.code32\n\t.code64\n#NO_APP\n\tnop\n\tmovq $1, %rax\n";
        let wanted = "#APP\n\t.code32\n\t.code64\n#NO_APP\n\tnop\n\tmovl $1, %eax\n";
        assert_eq!(rewrite(text.to_string(), Optimize::Faster), wanted);
    }

    /// What is written into the text assembles, as written, to the bytes
    /// the encoder chose for the statement it replaces.
    #[test]
    fn the_text_is_what_the_encoder_chose() {
        for (written, level) in [
            ("movq $1, %rax", Optimize::Faster),
            ("movabsq $0x12345678, %r9", Optimize::Faster),
            ("leaq (%rsp), %rsi", Optimize::Faster),
            // An address of three parts takes `lea` longer: for size only.
            ("leaq 8(,%rax,2), %rcx", Optimize::Smaller),
            ("movl 8(,%rax,2), %ecx", Optimize::Faster),
            ("movl (%r13,%rax), %ecx", Optimize::Faster),
            ("testq $0x40, %rdi", Optimize::Faster),
            ("movslq %eax, %rax", Optimize::Faster),
            ("movsd %xmm0, 16(%rsp)", Optimize::Faster),
            ("movdqu %xmm3, (%rdi,%rcx,8)", Optimize::Faster),
            ("xorpd %xmm1, %xmm1", Optimize::Faster),
            ("vpxord %zmm1, %zmm1, %zmm2", Optimize::Faster),
            ("{nf} addl $1, %ecx", Optimize::Faster),
            ("{nf} imull $5, %ecx, %edx", Optimize::Faster),
            ("movzbw %cl, %cx", Optimize::Smaller),
            ("movdqa %xmm1, %xmm2", Optimize::Smaller),
            ("movsd %xmm0, counter", Optimize::Faster),
            ("movdqu %xmm1, counter+16", Optimize::Faster),
            ("leaq counter, %rax", Optimize::Faster),
        ] {
            let text = rewrite(format!("\t{written}\n"), level);
            let statement = text.trim();
            assert_ne!(statement, written, "`{written}` was not replaced");
            let instr = parse_statement(written, level).expect(written);
            let mut encoder = InstructionEncoder::new();
            encoder.encode(&instr).expect(written);
            assert_eq!(
                bytes(statement),
                encoder.bytes,
                "`{written}` as `{statement}`"
            );
        }
    }

    #[test]
    fn thirty_two_bit_text() {
        let text = "\tleal (%esp), %esi\n\ttestl $1, %esi\n\tmovsd %xmm0, (%edi)\n\
                    \tmovsd %xmm1, counter\n";
        let wanted = "\tmovl %esp, %esi\n\ttestl $1, %esi\n\tmovlps %xmm0, (%edi)\n\
                      \tmovlps %xmm1, counter\n";
        assert_eq!(
            crate::backend::i686::assembler::rewrite_generated(text.to_string(), Optimize::Faster),
            wanted
        );
    }
}

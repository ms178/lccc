//! x86 `asm` operands in registers the back ends do not hand out.
//!
//! An `asm` operand in a register class is given a register by the back
//! end: a general register for `"r"`, an `xmm` register for `"x"`. Three
//! classes had no such machinery, and each went wrong in its own way:
//!
//! * A 256- or 512-bit vector under `"x"` or `"v"`. The operand was typed
//!   as a pointer, loaded with `movsd` into an `xmm` register and stored
//!   back eight bytes wide: `asm("vpaddd %2, %1, %0" : "=x"(r) : "x"(a),
//!   "x"(b))` with `__m256i` operands assembled `vpaddd %xmm2, %xmm1,
//!   %xmm0` over the wrong bits and returned garbage, at every `-O` level
//!   and without a word (GCC 16.2: `vpaddd %ymm1, %ymm0, %ymm0`).
//! * A mask register, `"k"` or `"Yk"`. `"k"` was refused ("impossible
//!   constraint in 'asm'"), `"Yk"` was given an `xmm` register and stopped
//!   in the assembler.
//! * An MMX register, `"y"`, was given a general register and stopped in
//!   the assembler.
//!
//! The registers of these classes are not allocated by this compiler, so
//! nothing has to be negotiated with an allocator: the statement itself
//! can load and store them. [`rewrite`] turns each such operand into one
//! the back ends do handle (memory, or a general register for a mask) and
//! writes the moves into the template:
//!
//! ```text
//! asm("vpaddd %2, %1, %0" : "=x"(r) : "x"(a), "x"(b));        /* __m256i */
//!
//! asm("vmovdqu %1, %%ymm14\n\t"
//!     "vmovdqu %2, %%ymm13\n\t"
//!     "vpaddd %%ymm13, %%ymm14, %%ymm15\n\t"
//!     "vmovdqu %%ymm15, %0"
//!     : "=m"(r) : "m"(a), "m"(b) : "xmm15", "xmm14", "xmm13");
//! ```
//!
//! A register is taken from the top of its file (`xmm15`, `k7`, `mm7`
//! downwards; the back ends hand out `xmm0` upwards), never one the
//! template or the clobber list names, and is added to the clobbers, which
//! keeps the back end's own choices for the other operands off it. A
//! statement that clobbers a register is a call point to the register
//! allocator (`liveness::x86_64_asm_stages_in_fixed_scratch`), so nothing
//! lives in the register across it.
//!
//! The price is the one a wide vector pays everywhere in an `asm`
//! statement of this compiler: its value is in memory before and behind
//! the statement. The vector intrinsics do not go this way.
//!
//! What the references in the template become is GCC's (`print_reg`): a
//! vector operand prints at its own width, `%x0`, `%t0` and `%g0` print the
//! `xmm`, `ymm` and `zmm` of the same register, a mask prints `%k1`.
//!
//! A mask moves through a general register with the `kmov` of its width.
//! `kmovb` belongs to AVX-512DQ, which a function that has only AVX-512F
//! may not use, so an eight-bit mask goes in with `kmovw` and has the
//! bits above it shifted out of the mask register: an eight-bit operand is
//! not known to be zero-extended in its general register. (GCC 16.2
//! without AVX-512DQ leaves those bits as they are.) An MMX operand moves
//! through a general register as well. In 32-bit code no general register
//! holds 64 bits, and both go through memory.

/// The register class of an operand this module pins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pinned {
    /// A vector of 32 or 64 bytes, in a `ymm` or `zmm` register.
    Wide { bytes: u8 },
    /// An integer of 1, 2, 4 or 8 bytes, in a mask register.
    Mask { bytes: u8 },
    /// 4 or 8 bytes in an MMX register.
    Mmx { bytes: u8 },
}

/// GCC's "inconsistent operand constraints in an 'asm'": the constraint
/// names register classes only, and none of them holds the operand's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Inconsistent;

/// What to do with an operand of one of the three classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    /// Give it a register of its class.
    Pin(Pinned),
    /// The constraint offers memory and no register that can hold the
    /// operand: memory it is.
    Memory,
    /// The constraint offers memory beside the register: memory, unless
    /// an input is tied to the operand, which needs the register.
    MemoryOrPin(Pinned),
}

/// One operand, in GCC's numbering: outputs, then inputs.
#[derive(Clone, Copy, Debug)]
pub(super) struct AsmOperandView<'a> {
    pub constraint: &'a str,
    pub name: Option<&'a str>,
    pub is_output: bool,
    /// [`classify`] of the operand's constraint and type.
    pub action: Option<Action>,
    /// The register of a register variable (`register __m256i v
    /// asm("ymm3")`): the operand is in this one.
    pub fixed: Option<u8>,
}

/// The statement after [`rewrite`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Rewritten {
    pub template: String,
    /// The constraint of each operand, in the same order.
    pub constraints: Vec<String>,
    /// The operands this module moved into memory. The source did not ask
    /// for an lvalue there, so a value that is none needs a temporary.
    pub in_memory: Vec<bool>,
    /// The registers taken, to add to the clobber list.
    pub clobbers: Vec<String>,
}

/// The letters of a constraint that choose a class: without the modifiers
/// in front, without the comment behind `#`, alternatives run together
/// (the back ends read a constraint as the union of its alternatives).
fn class_letters(constraint: &str) -> &str {
    let body = constraint.trim_start_matches(['=', '+', '&', '%']);
    body.split('#').next().unwrap_or(body)
}

/// What a constraint offers: every letter, with a `Y` pair read as one.
#[derive(Clone, Copy, Default)]
struct Offers {
    /// An SSE register: `x`, `v`, or a `Y` class of them.
    sse: bool,
    /// `Yz`, the first SSE register, and no other SSE class beside it.
    only_the_first_sse: bool,
    mask: bool,
    mmx: bool,
    general: bool,
    memory: bool,
    /// Anything else: an immediate, a tie, an x87 register, `X`, `p`.
    other: bool,
}

fn offers(constraint: &str) -> Offers {
    let mut found = Offers::default();
    let mut any_sse = false;
    let mut first_sse = false;
    let mut letters = class_letters(constraint).chars();
    while let Some(letter) = letters.next() {
        match letter {
            'x' | 'v' => any_sse = true,
            'k' => found.mask = true,
            'y' => found.mmx = true,
            'Y' => match letters.next() {
                Some('k') => found.mask = true,
                Some('z') => first_sse = true,
                // `Yv` and GCC's internal SSE classes; `Ym`, `Yn` and the
                // like are MMX-or-SSE classes the back ends read as SSE.
                Some(_) => any_sse = true,
                None => found.other = true,
            },
            'r' | 'q' | 'Q' | 'R' | 'l' | 'a' | 'b' | 'c' | 'd' | 'S' | 'D' | 'A' => {
                found.general = true;
            }
            'g' => {
                found.general = true;
                found.memory = true;
                found.other = true;
            }
            'm' | 'o' | 'V' => found.memory = true,
            // Alternatives are read as one; `*` and `?` weigh them.
            ',' | '*' | '?' | '!' | '^' | '$' => {}
            _ => found.other = true,
        }
    }
    found.sse = any_sse || first_sse;
    found.only_the_first_sse = first_sse && !any_sse;
    found
}

/// Whether an operand of this constraint and type is one of the three
/// classes, and what to do with it.
///
/// `vector` is the size of a vector type in bytes, `integer` the size of
/// an integer or pointer type. A constraint that offers a class the back
/// ends handle for this type (a general register for a mask or 64 bits)
/// is left to them: `Ok(None)`.
///
/// Which types a class holds is GCC 16.2's, measured: a mask register an
/// integer or a pointer of up to 64 bits; an MMX register 32 or 64 bits
/// of integer or a 64-bit vector; no general register a wide vector.
pub(super) fn classify(
    constraint: &str,
    vector: Option<usize>,
    integer: Option<usize>,
) -> Result<Option<Action>, Inconsistent> {
    if class_letters(constraint).starts_with('{') {
        // A register by name: the program's choice.
        return Ok(None);
    }
    let found = offers(constraint);
    let registers_only = !found.memory && !found.other;
    if let Some(bytes @ (32 | 64)) = vector {
        let wide = Pinned::Wide { bytes: bytes as u8 };
        return match (found.sse, found.memory) {
            (true, true) => Ok(Some(Action::MemoryOrPin(wide))),
            (true, false) => Ok(Some(Action::Pin(wide))),
            // `"g"`, `"rm"`: no general register holds it.
            (false, true) if found.general => Ok(Some(Action::Memory)),
            (false, true) => Ok(None),
            (false, false) if registers_only && (found.general || found.mask || found.mmx) => {
                Err(Inconsistent)
            }
            (false, false) => Ok(None),
        };
    }
    if found.general || found.sse {
        // The back ends take the general or the `xmm` register.
        return Ok(None);
    }
    let in_mask = match (vector, integer) {
        (None, Some(bytes @ (1 | 2 | 4 | 8))) if found.mask => {
            Some(Pinned::Mask { bytes: bytes as u8 })
        }
        _ => None,
    };
    let in_mmx = match (vector, integer) {
        (Some(8), _) if found.mmx => Some(Pinned::Mmx { bytes: 8 }),
        (None, Some(bytes @ (4 | 8))) if found.mmx => Some(Pinned::Mmx { bytes: bytes as u8 }),
        _ => None,
    };
    match (in_mask.or(in_mmx), found.memory) {
        // A mask beside memory is the back ends': they take the memory.
        (Some(Pinned::Mask { .. }), true) => Ok(None),
        (Some(pinned), true) => Ok(Some(Action::MemoryOrPin(pinned))),
        (Some(pinned), false) => Ok(Some(Action::Pin(pinned))),
        (None, _) if registers_only && (found.mask || found.mmx) => Err(Inconsistent),
        (None, _) => Ok(None),
    }
}

/// The register a register variable of a wide vector names, for
/// [`AsmOperandView::fixed`]: `xmm`, `ymm` or `zmm` and a number.
pub(super) fn sse_register_number(name: &str) -> Option<u8> {
    let name = name.trim_start_matches('%');
    let digits = ["xmm", "ymm", "zmm"].iter().find_map(|prefix| {
        name.get(..3)
            .filter(|head| head.eq_ignore_ascii_case(prefix))
            .map(|_| &name[3..])
    })?;
    if digits.is_empty() || digits.len() > 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|&n: &u8| n < 32)
}

/// GCC's words for a statement with more operands than registers.
const OUT_OF_REGISTERS: &str =
    "'asm' operand has impossible constraints or there are not enough registers";

/// The register files this module takes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum File {
    Sse,
    Mask,
    Mmx,
}

impl Pinned {
    fn file(self) -> File {
        match self {
            Pinned::Wide { .. } => File::Sse,
            Pinned::Mask { .. } => File::Mask,
            Pinned::Mmx { .. } => File::Mmx,
        }
    }

    /// The register `index` as a reference to this operand prints it:
    /// at the operand's own width, or at the width a modifier asks for.
    fn register(self, index: u8, modifier: Option<char>) -> String {
        match self {
            Pinned::Wide { bytes } => {
                let view = match (modifier, bytes) {
                    (Some('x'), _) => "xmm",
                    (Some('t'), _) => "ymm",
                    (Some('g'), _) => "zmm",
                    (_, 32) => "ymm",
                    _ => "zmm",
                };
                format!("{view}{index}")
            }
            Pinned::Mask { .. } => format!("k{index}"),
            Pinned::Mmx { .. } => format!("mm{index}"),
        }
    }

    /// The name the register is clobbered under.
    fn clobber(self, index: u8) -> String {
        match self {
            Pinned::Wide { .. } => format!("xmm{index}"),
            Pinned::Mask { .. } => format!("k{index}"),
            Pinned::Mmx { .. } => format!("mm{index}"),
        }
    }

    /// Whether the operand's own place is memory: a wide vector always,
    /// 64 bits where no general register holds them.
    fn through_memory(self, x86_64: bool) -> bool {
        match self {
            Pinned::Wide { .. } => true,
            Pinned::Mask { bytes } | Pinned::Mmx { bytes } => bytes == 8 && !x86_64,
        }
    }
}

/// The registers of each file a statement already names: bit `n` for
/// register `n`.
#[derive(Default)]
struct Busy {
    sse: u32,
    mask: u32,
    mmx: u32,
}

impl Busy {
    fn of(&mut self, file: File) -> &mut u32 {
        match file {
            File::Sse => &mut self.sse,
            File::Mask => &mut self.mask,
            File::Mmx => &mut self.mmx,
        }
    }

    /// Note a register name (`xmm3`, `ymm3`, `zmm3`, `k3`, `mm3`, in
    /// either case), with or without what follows it in a template.
    fn note(&mut self, name: &str) {
        for (prefix, file) in [
            ("xmm", File::Sse),
            ("ymm", File::Sse),
            ("zmm", File::Sse),
            ("mm", File::Mmx),
            ("k", File::Mask),
        ] {
            let Some(head) = name.get(..prefix.len()) else {
                continue;
            };
            if !head.eq_ignore_ascii_case(prefix) {
                continue;
            }
            let rest = &name[prefix.len()..];
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if let Ok(index) = rest[..digits].parse::<u32>() {
                if index < 32 {
                    *self.of(file) |= 1 << index;
                }
            }
            return;
        }
    }
}

/// One reference to an operand in a template: where it is, which operand,
/// and the modifier letter in front of it.
struct Reference {
    start: usize,
    end: usize,
    operand: usize,
    modifier: Option<char>,
}

/// The operand references of `template`, in order. The grammar is the one
/// the back ends substitute by (`x86_common::substitute_x86_asm_operands`):
/// `%%` is a percent sign, a modifier letter counts only in front of a
/// digit or a `[`, and anything else behind a `%` is not an operand.
fn references(template: &str, operands: &[AsmOperandView<'_>]) -> Vec<Reference> {
    let bytes = template.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'%' {
            at += 1;
            continue;
        }
        let start = at;
        at += 1;
        let Some(&next) = bytes.get(at) else { break };
        if next == b'%' {
            at += 1;
            continue;
        }
        let mut modifier = None;
        let follows = bytes.get(at + 1).copied();
        if matches!(
            next,
            b'k' | b'w'
                | b'b'
                | b'h'
                | b'q'
                | b'l'
                | b'c'
                | b'a'
                | b'n'
                | b'd'
                | b'x'
                | b't'
                | b'g'
                | b'P'
        ) && follows.is_some_and(|b| b.is_ascii_digit() || b == b'[')
        {
            modifier = Some(next as char);
            at += 1;
        }
        let operand = if bytes.get(at) == Some(&b'[') {
            let Some(close) = template[at..].find(']') else {
                break;
            };
            let name = &template[at + 1..at + close];
            at += close + 1;
            operands.iter().position(|op| op.name == Some(name))
        } else {
            let digits = bytes[at..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
            let number = template[at..at + digits].parse::<usize>().ok();
            at += digits;
            number.filter(|&n| n < operands.len())
        };
        if let Some(operand) = operand {
            found.push(Reference {
                start,
                end: at,
                operand,
                modifier,
            });
        }
    }
    found
}

/// The operand an input is tied to: a constraint that is a number, or the
/// name of an operand in brackets.
fn tied_to(constraint: &str, operands: &[AsmOperandView<'_>]) -> Option<usize> {
    let body = constraint.trim_start_matches('%');
    if let Some(name) = body
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        return operands.iter().position(|op| op.name == Some(name));
    }
    if body.is_empty() || !body.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    body.parse().ok().filter(|&n: &usize| n < operands.len())
}

/// The move of the operand numbered `operand` into the register of
/// `pinned` numbered `register` (`load`), or out of it.
fn transfer(pinned: Pinned, operand: usize, register: u8, load: bool, x86_64: bool) -> Vec<String> {
    let reg = format!("%%{}", pinned.register(register, None));
    let between = |mnemonic: &str, place: String| {
        if load {
            format!("{mnemonic} {place}, {reg}")
        } else {
            format!("{mnemonic} {reg}, {place}")
        }
    };
    let general = |modifier: char| format!("%{modifier}{operand}");
    let memory = || format!("%{operand}");
    match pinned {
        // `ymm16` and above have no VEX encoding.
        Pinned::Wide { bytes: 32 } if register < 16 => vec![between("vmovdqu", memory())],
        Pinned::Wide { .. } => vec![between("vmovdqu64", memory())],
        Pinned::Mmx { bytes: 4 } => vec![between("movd", general('k'))],
        Pinned::Mmx { .. } if x86_64 => vec![between("movq", general('q'))],
        Pinned::Mmx { .. } => vec![between("movq", memory())],
        Pinned::Mask { bytes: 1 } => {
            let mut lines = vec![between("kmovw", general('k'))];
            if load {
                lines.push(format!("kshiftlw $8, {reg}, {reg}"));
                lines.push(format!("kshiftrw $8, {reg}, {reg}"));
            }
            lines
        }
        Pinned::Mask { bytes: 2 } => vec![between("kmovw", general('k'))],
        Pinned::Mask { bytes: 4 } => vec![between("kmovd", general('k'))],
        Pinned::Mask { .. } if x86_64 => vec![between("kmovq", general('q'))],
        Pinned::Mask { .. } => vec![between("kmovq", memory())],
    }
}

/// `=`, `+` or nothing: the direction a constraint is written with.
fn direction(constraint: &str) -> &'static str {
    let modifiers = constraint.len() - constraint.trim_start_matches(['=', '+', '&', '%']).len();
    let modifiers = &constraint[..modifiers];
    if modifiers.contains('+') {
        "+"
    } else if modifiers.contains('=') {
        "="
    } else {
        ""
    }
}

/// Rewrite a statement whose operands [`classify`] has looked at.
/// `Ok(None)`: nothing to do, which is every statement without an operand
/// of the three classes. `leaves_by_label` is an `asm goto`.
pub(super) fn rewrite(
    template: &str,
    operands: &[AsmOperandView<'_>],
    clobbers: &[String],
    leaves_by_label: bool,
    x86_64: bool,
) -> Result<Option<Rewritten>, String> {
    if operands.iter().all(|op| op.action.is_none()) {
        return Ok(None);
    }
    // An input tied to an operand needs that operand in a register.
    let ties: Vec<Option<usize>> = operands
        .iter()
        .map(|op| {
            if op.is_output || op.action.is_some() {
                None
            } else {
                tied_to(op.constraint, operands)
            }
        })
        .collect();
    // The register class each operand is pinned in, if it is.
    let own: Vec<Option<Pinned>> = operands
        .iter()
        .enumerate()
        .map(|(at, op)| match op.action {
            Some(Action::Pin(pinned)) => Some(pinned),
            Some(Action::MemoryOrPin(pinned)) if ties.contains(&Some(at)) => Some(pinned),
            _ => None,
        })
        .collect();
    // The class of the register an input shares with what it is tied to.
    let tied: Vec<Option<(usize, Pinned)>> = ties
        .iter()
        .map(|to| to.and_then(|to| Some((to, own[to]?))))
        .collect();

    if leaves_by_label {
        // The stores stand behind the template, where a jump to a label
        // does not come by.
        if let Some(at) =
            (0..operands.len()).find(|&at| operands[at].is_output && own[at].is_some())
        {
            return Err(format!(
                "'asm goto' output operand {at} cannot be in a vector, mask or MMX register"
            ));
        }
    }

    let mut busy = Busy::default();
    for clobber in clobbers {
        busy.note(clobber.trim().trim_start_matches(['%', '#']));
    }
    for (at, _) in template.match_indices("%%") {
        busy.note(&template[at + 2..]);
    }

    let mut registers: Vec<Option<u8>> = vec![None; operands.len()];
    for (at, op) in operands.iter().enumerate() {
        if let (Some(pinned), Some(register)) = (own[at], op.fixed) {
            *busy.of(pinned.file()) |= 1 << register;
            registers[at] = Some(register);
        }
    }
    for (at, op) in operands.iter().enumerate() {
        let Some(pinned) = own[at] else { continue };
        if registers[at].is_some() {
            continue;
        }
        let file = pinned.file();
        let candidates: &[u8] = match file {
            File::Sse if offers(op.constraint).only_the_first_sse => &[0],
            File::Sse if x86_64 => &[15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0],
            File::Sse | File::Mmx => &[7, 6, 5, 4, 3, 2, 1, 0],
            // `k0` cannot mask an instruction; `"k"` may be asked to.
            File::Mask => &[7, 6, 5, 4, 3, 2, 1],
        };
        let taken = busy.of(file);
        let Some(&index) = candidates.iter().find(|&&n| *taken & (1 << n) == 0) else {
            return Err(OUT_OF_REGISTERS.to_string());
        };
        *taken |= 1 << index;
        registers[at] = Some(index);
    }

    // The register an operand is referred to by: its own, or the one of
    // the operand it is tied to.
    let place = |at: usize| -> Option<(Pinned, u8)> {
        match (own[at], tied[at]) {
            (Some(pinned), _) => Some((pinned, registers[at]?)),
            (None, Some((to, pinned))) => Some((pinned, registers[to]?)),
            (None, None) => None,
        }
    };

    let mut lines: Vec<String> = Vec::new();
    for (at, op) in operands.iter().enumerate() {
        let read = !op.is_output || direction(op.constraint) == "+";
        if let (true, Some((pinned, register))) = (read, place(at)) {
            lines.extend(transfer(pinned, at, register, true, x86_64));
        }
    }
    let mut body = String::with_capacity(template.len() + 16);
    let mut copied = 0;
    for reference in references(template, operands) {
        let Some((pinned, register)) = place(reference.operand) else {
            continue;
        };
        body.push_str(&template[copied..reference.start]);
        body.push_str("%%");
        body.push_str(&pinned.register(register, reference.modifier));
        copied = reference.end;
    }
    body.push_str(&template[copied..]);
    if !body.trim().is_empty() {
        lines.push(body);
    }
    for (at, op) in operands.iter().enumerate() {
        if let (true, Some(pinned), Some(register)) = (op.is_output, own[at], registers[at]) {
            lines.extend(transfer(pinned, at, register, false, x86_64));
        }
    }

    let mut constraints = Vec::with_capacity(operands.len());
    let mut in_memory = Vec::with_capacity(operands.len());
    for (at, op) in operands.iter().enumerate() {
        let memory = match (place(at), op.action) {
            (Some((pinned, _)), _) => Some(pinned.through_memory(x86_64)),
            (None, Some(_)) => Some(true),
            (None, None) => None,
        };
        constraints.push(match memory {
            Some(true) => format!("{}m", direction(op.constraint)),
            Some(false) => format!("{}r", direction(op.constraint)),
            None => op.constraint.to_string(),
        });
        in_memory.push(memory == Some(true));
    }
    let mut taken: Vec<String> = Vec::new();
    for at in 0..operands.len() {
        if let (Some(pinned), Some(register)) = (own[at], registers[at]) {
            let name = pinned.clobber(register);
            let listed = |clobber: &String| {
                clobber
                    .trim()
                    .trim_start_matches(['%', '#'])
                    .eq_ignore_ascii_case(&name)
            };
            if !clobbers.iter().any(listed) && !taken.contains(&name) {
                taken.push(name);
            }
        }
    }
    Ok(Some(Rewritten {
        template: lines.join("\n\t"),
        constraints,
        in_memory,
        clobbers: taken,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(bytes: u8) -> Option<Action> {
        Some(Action::Pin(Pinned::Wide { bytes }))
    }

    fn mask(bytes: u8) -> Option<Action> {
        Some(Action::Pin(Pinned::Mask { bytes }))
    }

    const MMX: Option<Action> = Some(Action::Pin(Pinned::Mmx { bytes: 8 }));

    fn op(constraint: &str, is_output: bool, action: Option<Action>) -> AsmOperandView<'_> {
        AsmOperandView {
            constraint,
            name: None,
            is_output,
            action,
            fixed: None,
        }
    }

    fn named<'a>(
        name: &'a str,
        constraint: &'a str,
        is_output: bool,
        action: Option<Action>,
    ) -> AsmOperandView<'a> {
        AsmOperandView {
            name: Some(name),
            ..op(constraint, is_output, action)
        }
    }

    fn rewritten_for(
        template: &str,
        operands: &[AsmOperandView<'_>],
        clobbers: &[&str],
        x86_64: bool,
    ) -> Rewritten {
        let clobbers: Vec<String> = clobbers.iter().map(|c| c.to_string()).collect();
        rewrite(template, operands, &clobbers, false, x86_64)
            .expect("the statement can be rewritten")
            .expect("the statement has an operand to pin")
    }

    fn rewritten(template: &str, operands: &[AsmOperandView<'_>], clobbers: &[&str]) -> Rewritten {
        rewritten_for(template, operands, clobbers, true)
    }

    #[test]
    fn a_wide_vector_under_an_sse_class_is_pinned() {
        for constraint in ["x", "=x", "+x", "v", "=&v", "Yz", "Yv", "%x", "x#comment"] {
            assert_eq!(
                classify(constraint, Some(32), None),
                Ok(wide(32)),
                "{constraint}"
            );
            assert_eq!(
                classify(constraint, Some(64), None),
                Ok(wide(64)),
                "{constraint}"
            );
            // 128 bits and less are the back ends' own.
            assert_eq!(
                classify(constraint, Some(16), None),
                Ok(None),
                "{constraint}"
            );
            assert_eq!(
                classify(constraint, None, Some(8)),
                Ok(None),
                "{constraint}"
            );
            assert_eq!(classify(constraint, None, None), Ok(None), "{constraint}");
        }
        for constraint in ["xm", "=vm", "x,m", "m,x", "xo", "xg"] {
            assert_eq!(
                classify(constraint, Some(32), None),
                Ok(Some(Action::MemoryOrPin(Pinned::Wide { bytes: 32 }))),
                "{constraint}"
            );
        }
        // No general register holds it: memory, where that is offered.
        for constraint in ["g", "rm", "=rm", "r,m"] {
            assert_eq!(
                classify(constraint, Some(64), None),
                Ok(Some(Action::Memory)),
                "{constraint}"
            );
        }
        for constraint in [
            "m", "=m", "o", "0", "i", "{xmm3}", "={ymm3}", "p", "X", "ri", "",
        ] {
            assert_eq!(
                classify(constraint, Some(32), None),
                Ok(None),
                "{constraint}"
            );
        }
        // GCC 16.2: "inconsistent operand constraints in an 'asm'".
        for constraint in ["r", "=r", "a", "q", "k", "Yk", "y", "ry"] {
            assert_eq!(
                classify(constraint, Some(32), None),
                Err(Inconsistent),
                "{constraint}"
            );
        }
    }

    #[test]
    fn a_mask_and_an_mmx_operand_are_pinned() {
        for constraint in ["k", "=k", "Yk", "=Yk", "+k", "=&Yk"] {
            for bytes in [1, 2, 4, 8] {
                assert_eq!(
                    classify(constraint, None, Some(bytes)),
                    Ok(mask(bytes as u8))
                );
            }
            // GCC 16.2 refuses a 128-bit integer, a vector and a double.
            assert_eq!(classify(constraint, Some(16), None), Err(Inconsistent));
            assert_eq!(classify(constraint, None, Some(16)), Err(Inconsistent));
            assert_eq!(classify(constraint, None, None), Err(Inconsistent));
        }
        // A general register or memory beside the mask: the back ends'.
        for constraint in ["kr", "rk", "Ykr", "km", "k,m", "kq", "kg"] {
            assert_eq!(
                classify(constraint, None, Some(2)),
                Ok(None),
                "{constraint}"
            );
            assert_eq!(classify(constraint, None, None), Ok(None), "{constraint}");
        }
        // An immediate beside it: the register is one of the two ways.
        assert_eq!(classify("ki", None, Some(2)), Ok(mask(2)));
        assert_eq!(classify("ki", None, None), Ok(None));
        for constraint in ["y", "=y", "+y", "=&y"] {
            assert_eq!(classify(constraint, Some(8), None), Ok(MMX));
            assert_eq!(classify(constraint, None, Some(8)), Ok(MMX));
            assert_eq!(
                classify(constraint, None, Some(4)),
                Ok(Some(Action::Pin(Pinned::Mmx { bytes: 4 })))
            );
            // GCC 16.2 refuses 16 bits, 128 bits and a double.
            assert_eq!(classify(constraint, None, Some(2)), Err(Inconsistent));
            assert_eq!(classify(constraint, Some(16), None), Err(Inconsistent));
            assert_eq!(classify(constraint, None, None), Err(Inconsistent));
        }
        assert_eq!(
            classify("ym", Some(8), None),
            Ok(Some(Action::MemoryOrPin(Pinned::Mmx { bytes: 8 })))
        );
        assert_eq!(classify("ym", None, Some(2)), Ok(None));
        assert_eq!(classify("yr", Some(8), None), Ok(None));
        assert_eq!(classify("yx", Some(8), None), Ok(None));
    }

    #[test]
    fn the_register_of_a_register_variable_is_read() {
        assert_eq!(sse_register_number("ymm3"), Some(3));
        assert_eq!(sse_register_number("%xmm15"), Some(15));
        assert_eq!(sse_register_number("ZMM31"), Some(31));
        for name in [
            "ymm32", "ymm", "ymm3x", "mm3", "k3", "rax", "xm", "", "ymm003",
        ] {
            assert_eq!(sse_register_number(name), None, "{name}");
        }
    }

    #[test]
    fn a_statement_without_such_an_operand_is_left_alone() {
        let operands = [
            op("=r", true, None),
            op("r", false, None),
            op("m", false, None),
        ];
        assert_eq!(
            rewrite("addl %1, %0", &operands, &[], false, true),
            Ok(None)
        );
        assert_eq!(rewrite("", &[], &[], true, true), Ok(None));
    }

    /// The statement of the module's header.
    #[test]
    fn three_vectors_are_loaded_computed_and_stored() {
        let operands = [
            op("=x", true, wide(32)),
            op("x", false, wide(32)),
            op("x", false, wide(32)),
        ];
        let got = rewritten("vpaddd %2, %1, %0", &operands, &[]);
        assert_eq!(
            got.template,
            "vmovdqu %1, %%ymm14\n\tvmovdqu %2, %%ymm13\n\t\
             vpaddd %%ymm13, %%ymm14, %%ymm15\n\tvmovdqu %%ymm15, %0"
        );
        assert_eq!(got.constraints, ["=m", "m", "m"]);
        assert_eq!(got.in_memory, [true, true, true]);
        assert_eq!(got.clobbers, ["xmm15", "xmm14", "xmm13"]);

        let operands = [op("=v", true, wide(64)), op("v", false, wide(64))];
        let got = rewritten("vpabsd %1, %0", &operands, &[]);
        assert_eq!(
            got.template,
            "vmovdqu64 %1, %%zmm14\n\tvpabsd %%zmm14, %%zmm15\n\tvmovdqu64 %%zmm15, %0"
        );
    }

    /// GCC's `print_reg`: `%x0`, `%t0` and `%g0` are the three widths of
    /// the operand's register, a bare reference its own.
    #[test]
    fn a_modifier_chooses_the_width_of_the_name() {
        let operands = [op("=x", true, wide(64)), op("x", false, wide(32))];
        let got = rewritten(
            "vinserti64x4 $1, %t1, %g0, %0\n\tvmovdqa %x1, %x0",
            &operands,
            &[],
        );
        assert_eq!(
            got.template,
            "vmovdqu %1, %%ymm14\n\t\
             vinserti64x4 $1, %%ymm14, %%zmm15, %%zmm15\n\tvmovdqa %%xmm14, %%xmm15\n\t\
             vmovdqu64 %%zmm15, %0"
        );
    }

    /// Operands by name, with and without a modifier; a name that is not
    /// an operand's and a goto label are not touched.
    #[test]
    fn named_references_are_replaced_too() {
        let operands = [
            named("out", "=x", true, wide(32)),
            named("in", "x", false, wide(32)),
        ];
        let got = rewritten(
            "vpabsd %[in], %[out]\n\tvmovdqa %x[in], %x[out]\n\tjmp %l[done]%[other]",
            &operands,
            &[],
        );
        assert_eq!(
            got.template,
            "vmovdqu %1, %%ymm14\n\t\
             vpabsd %%ymm14, %%ymm15\n\tvmovdqa %%xmm14, %%xmm15\n\tjmp %l[done]%[other]\n\t\
             vmovdqu %%ymm15, %0"
        );
    }

    /// What is not an operand reference stays as it is: a literal
    /// register, an escaped brace, a percent sign in front of a letter,
    /// a number that is no operand's.
    #[test]
    fn everything_else_behind_a_percent_sign_is_copied() {
        let operands = [
            op("=x", true, wide(64)),
            op("x", false, wide(64)),
            op("r", false, None),
        ];
        let got = rewritten(
            "vpaddd %%zmm3, %1, %0%{%%k1%}%{z%}\n\tmovl %2, %%eax # %7 %l3 %",
            &operands,
            &[],
        );
        assert_eq!(
            got.template,
            "vmovdqu64 %1, %%zmm14\n\t\
             vpaddd %%zmm3, %%zmm14, %%zmm15%{%%k1%}%{z%}\n\tmovl %2, %%eax # %7 %l3 %\n\t\
             vmovdqu64 %%zmm15, %0"
        );
        assert_eq!(got.constraints, ["=m", "m", "r"]);
        assert_eq!(got.in_memory, [true, true, false]);
    }

    /// A register the template or the clobber list names is not taken,
    /// under any of its three names and in either case.
    #[test]
    fn a_named_register_is_not_taken() {
        let operands = [op("=x", true, wide(32)), op("x", false, wide(32))];
        let got = rewritten(
            "vpaddd %%YMM15, %1, %0",
            &operands,
            &["xmm14", "%zmm13", "cc"],
        );
        assert!(
            got.template.contains("vpaddd %%YMM15, %%ymm11, %%ymm12"),
            "{}",
            got.template
        );
        assert_eq!(got.clobbers, ["xmm12", "xmm11"]);
        // 32-bit code has eight.
        let got = rewritten_for("vpabsd %1, %0", &operands, &["xmm7"], false);
        assert!(
            got.template.contains("vpabsd %%ymm5, %%ymm6"),
            "{}",
            got.template
        );
    }

    #[test]
    fn a_full_register_file_is_an_error() {
        let operands: Vec<_> = (0..9).map(|_| op("x", false, wide(32))).collect();
        assert_eq!(
            rewrite("", &operands, &[], false, false),
            Err(OUT_OF_REGISTERS.to_string())
        );
        let operands: Vec<_> = (0..8).map(|_| op("k", false, mask(2))).collect();
        assert_eq!(
            rewrite("", &operands, &[], false, true),
            Err(OUT_OF_REGISTERS.to_string())
        );
    }

    /// `"Yz"` is the first SSE register and no other.
    #[test]
    fn the_first_sse_register_is_the_first() {
        let operands = [op("=Yz", true, wide(32)), op("x", false, wide(32))];
        let got = rewritten("vpabsd %1, %0", &operands, &[]);
        assert!(
            got.template.contains("vpabsd %%ymm15, %%ymm0"),
            "{}",
            got.template
        );
        let clobbers = vec!["xmm0".to_string()];
        assert_eq!(
            rewrite("vpabsd %1, %0", &operands, &clobbers, false, true),
            Err(OUT_OF_REGISTERS.to_string())
        );
        // With another class beside it the choice is free again.
        let operands = [op("=Yzx", true, wide(32))];
        assert_eq!(rewritten("", &operands, &[]).clobbers, ["xmm15"]);
    }

    /// A register variable's operand is in the variable's register; the
    /// others stay off it. `ymm16` and above move with an EVEX encoding.
    #[test]
    fn a_register_variable_keeps_its_register() {
        let operands = [
            op("=x", true, wide(32)),
            AsmOperandView {
                fixed: Some(15),
                ..op("x", false, wide(32))
            },
            AsmOperandView {
                fixed: Some(20),
                ..op("v", false, wide(32))
            },
        ];
        let got = rewritten("vpaddd %2, %1, %0", &operands, &[]);
        assert_eq!(
            got.template,
            "vmovdqu %1, %%ymm15\n\tvmovdqu64 %2, %%ymm20\n\t\
             vpaddd %%ymm20, %%ymm15, %%ymm14\n\tvmovdqu %%ymm14, %0"
        );
        assert_eq!(got.clobbers, ["xmm14", "xmm15", "xmm20"]);
    }

    /// An input tied to a pinned output is loaded into the output's
    /// register, and a read-write output is loaded and stored.
    #[test]
    fn a_tied_input_and_a_read_write_output_are_loaded_into_the_output() {
        let operands = [
            op("=x", true, wide(32)),
            op("0", false, None),
            op("x", false, wide(32)),
        ];
        let got = rewritten("vpaddd %2, %0, %0\n\tvpaddd %1, %0, %0", &operands, &[]);
        assert_eq!(
            got.template,
            "vmovdqu %1, %%ymm15\n\tvmovdqu %2, %%ymm14\n\t\
             vpaddd %%ymm14, %%ymm15, %%ymm15\n\tvpaddd %%ymm15, %%ymm15, %%ymm15\n\t\
             vmovdqu %%ymm15, %0"
        );
        assert_eq!(got.constraints, ["=m", "m", "m"]);
        assert_eq!(got.clobbers, ["xmm15", "xmm14"]);

        let operands = [named("acc", "=x", true, wide(32)), op("[acc]", false, None)];
        let got = rewritten("vpabsd %1, %0", &operands, &[]);
        assert_eq!(
            got.template,
            "vmovdqu %1, %%ymm15\n\tvpabsd %%ymm15, %%ymm15\n\tvmovdqu %%ymm15, %0"
        );

        let operands = [op("+x", true, wide(64)), op("x", false, wide(64))];
        let got = rewritten("vpaddd %1, %0, %0", &operands, &[]);
        assert_eq!(
            got.template,
            "vmovdqu64 %0, %%zmm15\n\tvmovdqu64 %1, %%zmm14\n\t\
             vpaddd %%zmm14, %%zmm15, %%zmm15\n\tvmovdqu64 %%zmm15, %0"
        );
        assert_eq!(got.constraints, ["+m", "m"]);
        // A number that ties to an operand the back ends handle is theirs.
        let operands = [
            op("=r", true, None),
            op("0", false, None),
            op("x", false, wide(32)),
        ];
        let got = rewritten("x", &operands, &[]);
        assert_eq!(got.constraints, ["=r", "0", "m"]);
        assert_eq!(got.in_memory, [false, false, true]);
    }

    /// A mask moves through a general register with the `kmov` of its
    /// width; eight bits go in with `kmovw` and lose what is above them.
    #[test]
    fn a_mask_moves_through_a_general_register() {
        let operands = [
            op("=x", true, wide(64)),
            op("x", false, wide(64)),
            op("Yk", false, mask(2)),
        ];
        let got = rewritten("vpabsd %1, %0%{%2%}%{z%}", &operands, &[]);
        assert_eq!(
            got.template,
            "vmovdqu64 %1, %%zmm14\n\tkmovw %k2, %%k7\n\t\
             vpabsd %%zmm14, %%zmm15%{%%k7%}%{z%}\n\tvmovdqu64 %%zmm15, %0"
        );
        assert_eq!(got.constraints, ["=m", "m", "r"]);
        assert_eq!(got.in_memory, [true, true, false]);
        assert_eq!(got.clobbers, ["xmm15", "xmm14", "k7"]);

        let operands = [
            op("=k", true, mask(1)),
            op("k", false, mask(1)),
            op("k", false, mask(4)),
        ];
        let got = rewritten("kandw %2, %1, %0", &operands, &["k7"]);
        assert_eq!(
            got.template,
            "kmovw %k1, %%k5\n\tkshiftlw $8, %%k5, %%k5\n\tkshiftrw $8, %%k5, %%k5\n\t\
             kmovd %k2, %%k4\n\tkandw %%k4, %%k5, %%k6\n\tkmovw %%k6, %k0"
        );
        assert_eq!(got.constraints, ["=r", "r", "r"]);

        let operands = [op("=k", true, mask(8)), op("k", false, mask(8))];
        let got = rewritten("knotq %1, %0", &operands, &[]);
        assert_eq!(
            got.template,
            "kmovq %q1, %%k6\n\tknotq %%k6, %%k7\n\tkmovq %%k7, %q0"
        );
        // In 32-bit code no general register holds 64 bits: memory.
        let got = rewritten_for("knotq %1, %0", &operands, &[], false);
        assert_eq!(
            got.template,
            "kmovq %1, %%k6\n\tknotq %%k6, %%k7\n\tkmovq %%k7, %0"
        );
        assert_eq!(got.constraints, ["=m", "m"]);
        assert_eq!(got.in_memory, [true, true]);
        // An input tied to a mask is in a general register of its own.
        let operands = [
            op("+k", true, mask(2)),
            op("=k", true, mask(2)),
            op("1", false, None),
        ];
        let got = rewritten("korw %0, %1, %1", &operands, &[]);
        assert_eq!(
            got.template,
            "kmovw %k0, %%k7\n\tkmovw %k2, %%k6\n\tkorw %%k7, %%k6, %%k6\n\t\
             kmovw %%k7, %k0\n\tkmovw %%k6, %k1"
        );
        assert_eq!(got.constraints, ["+r", "=r", "r"]);
    }

    #[test]
    fn an_mmx_operand_moves_through_a_general_register() {
        let operands = [
            op("=&y", true, MMX),
            op("y", false, MMX),
            op("y", false, MMX),
        ];
        let got = rewritten("movq %1, %0\n\tpaddd %2, %0", &operands, &["mm7"]);
        assert_eq!(
            got.template,
            "movq %q1, %%mm5\n\tmovq %q2, %%mm4\n\t\
             movq %%mm5, %%mm6\n\tpaddd %%mm4, %%mm6\n\tmovq %%mm6, %q0"
        );
        assert_eq!(got.constraints, ["=r", "r", "r"]);
        assert_eq!(got.in_memory, [false, false, false]);
        assert_eq!(got.clobbers, ["mm6", "mm5", "mm4"]);

        // In 32-bit code no general register holds 64 bits: memory.
        let got = rewritten_for("movq %1, %0\n\tpaddd %2, %0", &operands, &[], false);
        assert_eq!(
            got.template,
            "movq %1, %%mm6\n\tmovq %2, %%mm5\n\t\
             movq %%mm6, %%mm7\n\tpaddd %%mm5, %%mm7\n\tmovq %%mm7, %0"
        );
        assert_eq!(got.constraints, ["=m", "m", "m"]);
        assert_eq!(got.in_memory, [true, true, true]);

        // 32 bits fit one in either mode.
        let half = Some(Action::Pin(Pinned::Mmx { bytes: 4 }));
        let operands = [op("=y", true, half), op("y", false, half)];
        for x86_64 in [true, false] {
            let got = rewritten_for("pshufw $27, %1, %0", &operands, &[], x86_64);
            assert_eq!(
                got.template,
                "movd %k1, %%mm6\n\tpshufw $27, %%mm6, %%mm7\n\tmovd %%mm7, %k0"
            );
            assert_eq!(got.constraints, ["=r", "r"]);
        }
    }

    /// A constraint that offers memory beside the register is narrowed to
    /// memory and the template is not touched, unless an input is tied to
    /// the operand: that takes the register.
    #[test]
    fn memory_is_taken_where_it_is_offered() {
        let either = Some(Action::MemoryOrPin(Pinned::Wide { bytes: 32 }));
        let operands = [op("=xm", true, either), op("x", false, wide(32))];
        let got = rewritten("vmovdqu %1, %0", &operands, &[]);
        assert_eq!(got.template, "vmovdqu %1, %%ymm15\n\tvmovdqu %%ymm15, %0");
        assert_eq!(got.constraints, ["=m", "m"]);
        assert_eq!(got.clobbers, ["xmm15"]);

        let operands = [op("=xm", true, either), op("0", false, None)];
        let got = rewritten("vpabsd %0, %0", &operands, &[]);
        assert_eq!(
            got.template,
            "vmovdqu %1, %%ymm15\n\tvpabsd %%ymm15, %%ymm15\n\tvmovdqu %%ymm15, %0"
        );

        let operands = [op("=r", true, None), op("g", false, Some(Action::Memory))];
        let got = rewritten("movl %1, %0", &operands, &[]);
        assert_eq!(got.template, "movl %1, %0");
        assert_eq!(got.constraints, ["=r", "m"]);
        assert_eq!(got.in_memory, [false, true]);
        assert!(got.clobbers.is_empty());
    }

    /// The stores stand behind the template; a jump out of it would skip
    /// them. Inputs are loaded in front of it and are no trouble.
    #[test]
    fn an_asm_goto_output_is_refused() {
        let operands = [op("=x", true, wide(32)), op("x", false, wide(32))];
        assert_eq!(
            rewrite("vpabsd %1, %0", &operands, &[], true, true),
            Err(
                "'asm goto' output operand 0 cannot be in a vector, mask or MMX register"
                    .to_string()
            )
        );
        let operands = [op("x", false, wide(32))];
        let got = rewrite("vptest %0, %0\n\tjz %l1", &operands, &[], true, true);
        assert_eq!(
            got.unwrap().unwrap().template,
            "vmovdqu %0, %%ymm15\n\tvptest %%ymm15, %%ymm15\n\tjz %l1"
        );
    }
}

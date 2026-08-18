//! GNU linker script engine (parser + expression evaluator).
//!
//! Implements the subset of the GNU ld script language required to link the
//! Linux kernel's `vmlinux.lds` (and similar full scripts):
//!
//! * `ENTRY(sym)`, `OUTPUT_FORMAT(...)`, `OUTPUT_ARCH(...)` (latter two ignored)
//! * top-level symbol assignments and aliases (`jiffies = jiffies_64;`)
//! * `PHDRS { name PT_LOAD FLAGS(5); ... }`
//! * `SECTIONS { ... }` with:
//!   - location counter assignment `. = expr;`, `. += expr;`
//!   - symbol assignments `sym = expr;` and `PROVIDE(sym = expr);`
//!   - output sections `NAME [addr] [(INFO)] : [AT(expr)] { ... } [:phdr]* [= fill]`
//!   - input specs `*(pat...)`, `KEEP(*(...))`, `SORT(pat)`, `SORT_BY_ALIGNMENT(pat)`
//!   - `/DISCARD/ : { ... }`
//!   - in-section `. = ALIGN(n);`, symbol captures, `ASSERT(...)`, `CONSTRUCTORS`
//! * expressions: numbers, symbols, `.`, arithmetic/bitwise/shift/compare ops,
//!   `ALIGN(x)`, `ALIGN(a,b)`, `ABSOLUTE(e)`, `ADDR(sec)`, `SIZEOF(sec)`,
//!   `ASSERT(e, "msg")`, `MIN`/`MAX`.
//!
//! The evaluator is deliberately separate from layout: the x86 emitter drives
//! it with callbacks for symbol and section lookups so evaluation order and
//! deferred (forward-referencing) assignments are handled by the layout engine.

use crate::common::fx_hash::FxHashMap;

// ── AST ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Expr {
    /// `SEGMENT_START("name", default)` -- overridable segment base.
    SegmentStart(String, Box<Expr>),
    Num(u64),
    Sym(String),
    Dot,
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    /// ALIGN(x): align the location counter to x. ALIGN(a, b): align a to b.
    Align1(Box<Expr>),
    Align2(Box<Expr>, Box<Expr>),
    Absolute(Box<Expr>),
    AddrOf(String),
    LoadAddrOf(String),
    SizeOf(String),
    AlignOf(String),
    DefinedSym(String),
    Min(Box<Expr>, Box<Expr>),
    Max(Box<Expr>, Box<Expr>),
    Assert(Box<Expr>, String),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp { Add, Sub, Mul, Div, Rem, Shl, Shr, And, Or, Xor,
                 Eq, Ne, Lt, Le, Gt, Ge, LAnd, LOr }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AssignOp { Set, Add }

#[derive(Debug, Clone)]
pub struct Assignment {
    pub symbol: String,           // "." for the location counter
    pub op: AssignOp,
    pub expr: Expr,
    pub provide: bool,
    /// `PROVIDE_HIDDEN`/`HIDDEN`: the symbol is defined with STV_HIDDEN, so it
    /// must not appear in `.dynsym` and cannot be preempted. Dropping this bit
    /// silently exports internal markers (`__init_array_start` and friends)
    /// from every shared object that uses a custom script.
    pub hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SortKind { None, ByName, ByAlignment, ByInitPriority }

#[derive(Debug, Clone)]
pub struct InputSpec {
    /// Section name glob patterns (`.text.*`, `___ksymtab+*`, ...).
    pub patterns: Vec<String>,
    pub keep: bool,
    pub sort: SortKind,
}

#[derive(Debug, Clone)]
pub enum SecItem {
    Input(InputSpec),
    Assign(Assignment),
    Assert(Expr, String),
    /// Emit a scalar data item at the current location counter.  The width is
    /// in bytes and is one of 1/2/4/8 for BYTE/SHORT/LONG/QUAD respectively.
    /// Keeping the expression (rather than folding constants in the parser)
    /// is required for constructs such as `LONG(symbol - .)`.
    Data { width: u8, expr: Expr },
    /// CONSTRUCTORS keyword (legacy; no-op for ELF)
    Constructors,
}

#[derive(Debug, Clone)]
pub struct OutputSecDef {
    pub name: String,
    /// Explicit start address expression (e.g. the `0` on debug sections).
    pub address: Option<Expr>,
    /// `(INFO)` / `(NOLOAD)` type marker -> section not allocated.
    pub info: bool,
    /// `AT(expr)` load address (LMA).
    pub at_lma: Option<Expr>,
    pub items: Vec<SecItem>,
    /// `:phdr` assignments after the closing brace.
    pub phdrs: Vec<String>,
    /// `= 0xNN` fill pattern.
    pub fill: Option<u64>,
    /// ALIGN(..) on the section header line (`.foo : ALIGN(8) { ... }`).
    pub align: Option<Expr>,
    /// `> region` -- the VMA region this section is placed in.
    pub region: Option<String>,
    /// `AT> region` -- the LMA region (load address), for ROM-resident data.
    pub lma_region: Option<String>,
}

#[derive(Debug, Clone)]
pub enum SectionsItem {
    Assign(Assignment),
    Assert(Expr, String),
    Output(OutputSecDef),
}

#[derive(Debug, Clone)]
pub struct PhdrDecl {
    pub name: String,
    pub ptype: u32,
    pub flags: Option<u64>,
    pub has_phdrs: bool,   // PHDRS keyword attribute
    pub has_filehdr: bool, // FILEHDR keyword attribute
}

/// A `MEMORY { name (attrs) : ORIGIN = a, LENGTH = n }` region.
///
/// Regions give a script author a declarative way to say "this section lives
/// in SRAM and SRAM is 64 KiB". The value is not the placement -- an explicit
/// address does that too -- but the *overflow check*: running out of a region
/// is diagnosed by name instead of producing an image that silently overlaps.
#[derive(Debug, Clone)]
pub struct MemoryRegion {
    pub name: String,
    pub origin: Expr,
    pub length: Expr,
    /// Attribute letters between the parentheses (`rwx`, `!r`, ...). Retained
    /// verbatim; ELF output has no place for them, but they must parse.
    pub attrs: String,
}

#[derive(Debug, Clone, Default)]
pub struct LinkerScript {
    pub entry: Option<String>,
    pub phdrs: Vec<PhdrDecl>,
    pub sections: Vec<SectionsItem>,
    /// `MEMORY { ... }` regions, in declaration order.
    pub memory: Vec<MemoryRegion>,
    /// Assignments and asserts appearing outside SECTIONS (evaluated after layout
    /// if they reference section-defined symbols).
    pub top_assigns: Vec<Assignment>,
    pub top_asserts: Vec<(Expr, String)>,
}

// ── Tokenizer ───────────────────────────────────────────────────────────

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),      // includes section names like .text..foo, /DISCARD/
    Num(u64),
    Str(String),
    Punct(&'static str),
    Eof,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self { Lexer { src: src.as_bytes(), pos: 0 } }

    fn skip_ws(&mut self) {
        loop {
            while self.pos < self.src.len() && (self.src[self.pos] as char).is_whitespace() {
                self.pos += 1;
            }
            // /* ... */ comments
            if self.pos + 1 < self.src.len() && self.src[self.pos] == b'/' && self.src[self.pos+1] == b'*' {
                self.pos += 2;
                while self.pos + 1 < self.src.len()
                    && !(self.src[self.pos] == b'*' && self.src[self.pos+1] == b'/') {
                    self.pos += 1;
                }
                self.pos = (self.pos + 2).min(self.src.len());
                continue;
            }
            break;
        }
    }

    fn is_ident_char(c: u8, first: bool) -> bool {
        // Section/symbol/pattern characters. `/DISCARD/` and `*.o`-style file
        // patterns are also lexed as identifiers.
        match c {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'$' => true,
            b'*' | b'?' | b'[' | b']' | b'\\' | b'-' | b'+' if !first => false, // handled by pattern lexer
            // `/DISCARD/` is recognised explicitly in `next`; every other
            // slash is the arithmetic division operator.
            b'/' => false,
            _ => false,
        }
    }

    /// Lex the next token in normal (expression/statement) context.
    fn next(&mut self) -> Tok {
        self.skip_ws();
        if self.pos >= self.src.len() { return Tok::Eof; }
        let c = self.src[self.pos];

        // string literal
        if c == b'"' {
            self.pos += 1;
            let start = self.pos;
            while self.pos < self.src.len() && self.src[self.pos] != b'"' { self.pos += 1; }
            let s = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
            self.pos = (self.pos + 1).min(self.src.len());
            return Tok::Str(s);
        }

        // number
        if c.is_ascii_digit() {
            let start = self.pos;
            if c == b'0' && self.pos + 1 < self.src.len()
                && (self.src[self.pos+1] | 0x20) == b'x' {
                self.pos += 2;
                while self.pos < self.src.len() && self.src[self.pos].is_ascii_hexdigit() { self.pos += 1; }
                let v = u64::from_str_radix(
                    &String::from_utf8_lossy(&self.src[start+2..self.pos]), 16).unwrap_or(0);
                return Tok::Num(self.num_suffix(v));
            }
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() { self.pos += 1; }
            let v: u64 = String::from_utf8_lossy(&self.src[start..self.pos]).parse().unwrap_or(0);
            return Tok::Num(self.num_suffix(v));
        }

        // `/DISCARD/` is the one slash-delimited identifier in GNU scripts.
        // Recognise the complete token here so an ordinary slash remains the
        // division operator handled by the punctuation table below.
        if self.src[self.pos..].starts_with(b"/DISCARD/") {
            self.pos += b"/DISCARD/".len();
            return Tok::Ident("/DISCARD/".to_string());
        }

        // identifier / section name / keyword
        if Self::is_ident_char(c, true) {
            let start = self.pos;
            self.pos += 1;
            while self.pos < self.src.len() && Self::is_ident_char(self.src[self.pos], false) {
                self.pos += 1;
            }
            return Tok::Ident(String::from_utf8_lossy(&self.src[start..self.pos]).into_owned());
        }

        // punctuation / operators (longest match first)
        let two: &[(&[u8], &'static str)] = &[
            (b"<<", "<<"), (b">>", ">>"), (b"<=", "<="), (b">=", ">="),
            (b"==", "=="), (b"!=", "!="), (b"&&", "&&"), (b"||", "||"),
            (b"+=", "+="), (b"-=", "-="),
        ];
        for (pat, tok) in two {
            if self.src[self.pos..].starts_with(pat) {
                self.pos += 2;
                return Tok::Punct(tok);
            }
        }
        let one: &[(u8, &'static str)] = &[
            (b'{', "{"), (b'}', "}"), (b'(', "("), (b')', ")"),
            (b';', ";"), (b':', ":"), (b',', ","), (b'=', "="),
            (b'+', "+"), (b'-', "-"), (b'*', "*"), (b'/', "/"), (b'%', "%"),
            (b'&', "&"), (b'|', "|"), (b'^', "^"), (b'~', "~"), (b'!', "!"),
            (b'<', "<"), (b'>', ">"), (b'?', "?"),
        ];
        for (ch, tok) in one {
            if c == *ch { self.pos += 1; return Tok::Punct(tok); }
        }
        // Unknown byte: skip it
        self.pos += 1;
        self.next()
    }

    fn num_suffix(&mut self, v: u64) -> u64 {
        if self.pos < self.src.len() {
            match self.src[self.pos] | 0x20 {
                b'k' => { self.pos += 1; return v << 10; }
                b'm' => { self.pos += 1; return v << 20; }
                _ => {}
            }
        }
        v
    }

    /// Lex a section-name glob pattern: everything up to whitespace or ')'.
    fn next_pattern(&mut self) -> Option<String> {
        self.skip_ws();
        if self.pos >= self.src.len() { return None; }
        if self.src[self.pos] == b')' { return None; }
        let start = self.pos;
        while self.pos < self.src.len() {
            let c = self.src[self.pos];
            if (c as char).is_whitespace() || c == b')' { break; }
            self.pos += 1;
        }
        if self.pos == start { return None; }
        Some(String::from_utf8_lossy(&self.src[start..self.pos]).into_owned())
    }

    fn save(&self) -> usize { self.pos }
    fn restore(&mut self, p: usize) { self.pos = p; }
}

// ── Parser ──────────────────────────────────────────────────────────────

pub fn parse_linker_script(src: &str) -> Result<LinkerScript, String> {
    let mut lx = Lexer::new(src);
    let mut script = LinkerScript::default();

    loop {
        let save = lx.save();
        match lx.next() {
            Tok::Eof => break,
            Tok::Ident(kw) => match kw.as_str() {
                "ENTRY" => {
                    expect(&mut lx, "(")?;
                    if let Tok::Ident(sym) = lx.next() { script.entry = Some(sym); }
                    expect(&mut lx, ")")?;
                }
                "OUTPUT_FORMAT" | "OUTPUT_ARCH" | "TARGET" | "SEARCH_DIR" | "OUTPUT" => {
                    skip_parens(&mut lx)?;
                }
                "INCLUDE" => { let _ = lx.next(); }
                "PHDRS" => {
                    expect(&mut lx, "{")?;
                    loop {
                        match lx.next() {
                            Tok::Punct("}") => break,
                            Tok::Ident(name) => {
                                let mut decl = PhdrDecl {
                                    name, ptype: 0, flags: None,
                                    has_phdrs: false, has_filehdr: false,
                                };
                                loop {
                                    match lx.next() {
                                        Tok::Punct(";") => break,
                                        Tok::Ident(a) => match a.as_str() {
                                            "PT_LOAD" => decl.ptype = 1,
                                            "PT_DYNAMIC" => decl.ptype = 2,
                                            "PT_INTERP" => decl.ptype = 3,
                                            "PT_NOTE" => decl.ptype = 4,
                                            "PT_PHDR" => decl.ptype = 6,
                                            "PT_TLS" => decl.ptype = 7,
                                            "PT_GNU_EH_FRAME" => decl.ptype = 0x6474_e550,
                                            "PT_GNU_STACK" => decl.ptype = 0x6474_e551,
                                            "PT_GNU_RELRO" => decl.ptype = 0x6474_e552,
                                            "PT_GNU_PROPERTY" => decl.ptype = 0x6474_e553,
                                            "PT_GNU_SFRAME" => decl.ptype = 0x6474_e554,
                                            "PHDRS" => decl.has_phdrs = true,
                                            "FILEHDR" => decl.has_filehdr = true,
                                            "FLAGS" => {
                                                expect(&mut lx, "(")?;
                                                let e = parse_expr(&mut lx)?;
                                                expect(&mut lx, ")")?;
                                                decl.flags = Some(eval_const(&e).unwrap_or(0));
                                            }
                                            "AT" => { skip_parens(&mut lx)?; }
                                            _ => {}
                                        },
                                        Tok::Num(n) => decl.ptype = n as u32,
                                        Tok::Eof => return Err("unterminated PHDRS".into()),
                                        _ => {}
                                    }
                                }
                                script.phdrs.push(decl);
                            }
                            Tok::Eof => return Err("unterminated PHDRS".into()),
                            _ => {}
                        }
                    }
                }
                "SECTIONS" => {
                    expect(&mut lx, "{")?;
                    parse_sections_body(&mut lx, &mut script.sections)?;
                }
                "MEMORY" => {
                    expect(&mut lx, "{")?;
                    parse_memory_body(&mut lx, &mut script.memory)?;
                }
                "ASSERT" => {
                    let (e, msg) = parse_assert_args(&mut lx)?;
                    script.top_asserts.push((e, msg));
                    eat_semi(&mut lx);
                }
                "PROVIDE" | "PROVIDE_HIDDEN" | "HIDDEN" => {
                    expect(&mut lx, "(")?;
                    // HIDDEN(x = e) defines unconditionally; PROVIDE* only if
                    // the symbol is still undefined.
                    let a = parse_assignment_inner(
                        &mut lx, kw != "HIDDEN", kw != "PROVIDE")?;
                    expect(&mut lx, ")")?;
                    eat_semi(&mut lx);
                    script.top_assigns.push(a);
                }
                _ => {
                    // top-level assignment: sym = expr;  or  . = ASSERT(...)
                    lx.restore(save);
                    if let Some(a) = try_parse_assignment(&mut lx)? {
                        script.top_assigns.push(a);
                    } else {
                        // Unknown construct: skip one token to make progress
                        let _ = lx.next();
                    }
                }
            },
            Tok::Punct(".") | Tok::Punct(_) => {
                lx.restore(save);
                if let Some(a) = try_parse_assignment(&mut lx)? {
                    script.top_assigns.push(a);
                } else {
                    let _ = lx.next();
                }
            }
            _ => {}
        }
    }
    Ok(script)
}

fn expect(lx: &mut Lexer, p: &str) -> Result<(), String> {
    match lx.next() {
        Tok::Punct(q) if q == p => Ok(()),
        t => Err(format!("linker script: expected '{}', got {:?}", p, t)),
    }
}

fn eat_semi(lx: &mut Lexer) {
    let save = lx.save();
    if lx.next() != Tok::Punct(";") { lx.restore(save); }
}

fn skip_parens(lx: &mut Lexer) -> Result<(), String> {
    expect(lx, "(")?;
    let mut depth = 1;
    loop {
        match lx.next() {
            Tok::Punct("(") => depth += 1,
            Tok::Punct(")") => { depth -= 1; if depth == 0 { return Ok(()); } }
            Tok::Eof => return Err("unterminated parens".into()),
            _ => {}
        }
    }
}

fn parse_assert_args(lx: &mut Lexer) -> Result<(Expr, String), String> {
    expect(lx, "(")?;
    let e = parse_expr(lx)?;
    let mut msg = String::new();
    let save = lx.save();
    if lx.next() == Tok::Punct(",") {
        if let Tok::Str(s) = lx.next() { msg = s; }
    } else { lx.restore(save); }
    expect(lx, ")")?;
    Ok((e, msg))
}

/// Try to parse `sym = expr;` / `. = expr;` / `sym += expr;`; returns None and
/// restores position when the lookahead is not an assignment.
fn try_parse_assignment(lx: &mut Lexer) -> Result<Option<Assignment>, String> {
    let save = lx.save();
    let name = match lx.next() {
        Tok::Ident(n) => n,
        _ => { lx.restore(save); return Ok(None); }
    };
    let op = match lx.next() {
        Tok::Punct("=") => AssignOp::Set,
        Tok::Punct("+=") => AssignOp::Add,
        _ => { lx.restore(save); return Ok(None); }
    };
    let expr = parse_expr(lx)?;
    eat_semi(lx);
    Ok(Some(Assignment { symbol: name, op, expr, provide: false, hidden: false }))
}

fn parse_assignment_inner(lx: &mut Lexer, provide: bool, hidden: bool)
    -> Result<Assignment, String>
{
    let name = match lx.next() {
        Tok::Ident(n) => n,
        t => return Err(format!("expected symbol in PROVIDE/HIDDEN, got {:?}", t)),
    };
    expect(lx, "=")?;
    let expr = parse_expr(lx)?;
    Ok(Assignment { symbol: name, op: AssignOp::Set, expr, provide, hidden })
}

/// Parse a `MEMORY { name (attrs) : ORIGIN = expr, LENGTH = expr  ... }` body.
///
/// GNU ld accepts `org`/`o` and `len`/`l` as abbreviations, and allows the
/// attribute list to be omitted entirely. Both are accepted here because real
/// firmware scripts use them interchangeably.
fn parse_memory_body(lx: &mut Lexer, out: &mut Vec<MemoryRegion>) -> Result<(), String> {
    loop {
        match lx.next() {
            Tok::Punct("}") => return Ok(()),
            Tok::Eof => return Err("unterminated MEMORY block".to_string()),
            Tok::Ident(name) => {
                let mut attrs = String::new();
                let save = lx.save();
                if lx.next() == Tok::Punct("(") {
                    // Attribute letters, possibly several tokens (`!`, `rwx`).
                    loop {
                        match lx.next() {
                            Tok::Punct(")") => break,
                            Tok::Eof => return Err("unterminated MEMORY attributes".into()),
                            Tok::Ident(a) => attrs.push_str(&a),
                            Tok::Punct(p) => attrs.push_str(p),
                            t => return Err(format!("bad MEMORY attribute {:?}", t)),
                        }
                    }
                } else {
                    lx.restore(save);
                }
                expect(lx, ":")?;
                let mut origin = None;
                let mut length = None;
                // ORIGIN = e , LENGTH = e   (either order, comma optional)
                for _ in 0..2 {
                    let save = lx.save();
                    match lx.next() {
                        Tok::Ident(k) => {
                            let key = k.to_ascii_uppercase();
                            expect(lx, "=")?;
                            let e = parse_expr(lx)?;
                            if key == "ORIGIN" || key == "ORG" || key == "O" {
                                origin = Some(e);
                            } else if key == "LENGTH" || key == "LEN" || key == "L" {
                                length = Some(e);
                            } else {
                                return Err(format!("unknown MEMORY key '{}'", k));
                            }
                            let save2 = lx.save();
                            if lx.next() != Tok::Punct(",") { lx.restore(save2); }
                        }
                        _ => { lx.restore(save); break; }
                    }
                }
                let (Some(origin), Some(length)) = (origin, length) else {
                    return Err(format!("MEMORY region '{}' needs both ORIGIN and LENGTH", name));
                };
                out.push(MemoryRegion { name, origin, length, attrs });
            }
            t => return Err(format!("unexpected token in MEMORY block: {:?}", t)),
        }
    }
}

fn parse_sections_body(lx: &mut Lexer, out: &mut Vec<SectionsItem>) -> Result<(), String> {
    loop {
        let save = lx.save();
        match lx.next() {
            Tok::Punct("}") => return Ok(()),
            Tok::Eof => return Err("unterminated SECTIONS".into()),
            Tok::Ident(kw) if kw == "ASSERT" => {
                let (e, msg) = parse_assert_args(lx)?;
                eat_semi(lx);
                out.push(SectionsItem::Assert(e, msg));
            }
            Tok::Ident(kw) if kw == "PROVIDE" || kw == "PROVIDE_HIDDEN" || kw == "HIDDEN" => {
                expect(lx, "(")?;
                let a = parse_assignment_inner(lx, kw != "HIDDEN", kw != "PROVIDE")?;
                expect(lx, ")")?;
                eat_semi(lx);
                out.push(SectionsItem::Assign(a));
            }
            Tok::Ident(kw) if kw == "OVERLAY" => parse_overlay(lx, out)?,
            Tok::Ident(_name) => {
                lx.restore(save);
                // assignment or output section definition?
                if let Some(a) = try_parse_assignment(lx)? {
                    out.push(SectionsItem::Assign(a));
                    continue;
                }
                let def = parse_output_section(lx)?;
                out.push(SectionsItem::Output(def));
            }
            _ => { /* stray token */ }
        }
    }
}

/// Parse the `{ ... }` body of an output section into `def`.
///
/// Shared by ordinary output sections and OVERLAY members: the body
/// grammar is identical, and duplicating it would let the two drift.
/// Parse `OVERLAY [start] : [AT(lma)] { .a { ... } .b { ... } } [>region]`.
///
/// Overlay members all share one VMA -- they are alternative contents for the
/// same address range, swapped in at runtime -- while their load addresses run
/// consecutively. GNU ld also defines `__load_start_<name>` and
/// `__load_stop_<name>` so the copy routine can locate the bytes; an overlay is
/// unusable without them, so they are emitted here too.
///
/// The construct is desugared into ordinary output sections: each member gets
/// an explicit address equal to the overlay start and an `AT()` naming its own
/// slot in the load region. Layout downstream needs no concept of overlays.
fn parse_overlay(lx: &mut Lexer, out: &mut Vec<SectionsItem>) -> Result<(), String> {
    let mut start: Option<Expr> = None;
    loop {
        let save = lx.save();
        match lx.next() {
            Tok::Punct(":") => break,
            Tok::Eof => return Err("unterminated OVERLAY header".into()),
            _ => { lx.restore(save); start = Some(parse_expr(lx)?); }
        }
    }
    let mut lma: Option<Expr> = None;
    loop {
        let save = lx.save();
        match lx.next() {
            Tok::Punct("{") => break,
            Tok::Ident(k) if k == "AT" => {
                expect(lx, "(")?;
                lma = Some(parse_expr(lx)?);
                expect(lx, ")")?;
            }
            Tok::Ident(k) if k == "NOCROSSREFS" => {}
            t => return Err(format!("unexpected token in OVERLAY header: {:?}", t)),
        }
    }

    let base = start.unwrap_or(Expr::Dot);
    let mut lma_off: Option<Expr> = None;
    loop {
        let save = lx.save();
        match lx.next() {
            Tok::Punct("}") => break,
            Tok::Eof => return Err("unterminated OVERLAY body".into()),
            Tok::Ident(name) => {
                let _ = save;
                expect(lx, "{")?;
                let mut def = OutputSecDef {
                    name: name.clone(), address: Some(base.clone()), info: false,
                    at_lma: None, items: Vec::new(), phdrs: Vec::new(),
                    fill: None, align: None, region: None, lma_region: None,
                };
                if let Some(ref l) = lma {
                    def.at_lma = Some(match lma_off {
                        None => l.clone(),
                        Some(ref off) => Expr::Bin(BinOp::Add,
                            Box::new(l.clone()), Box::new(off.clone())),
                    });
                }
                parse_output_section_body(lx, &mut def)?;
                let this_size = Expr::SizeOf(name.clone());
                lma_off = Some(match lma_off {
                    None => this_size,
                    Some(off) => Expr::Bin(BinOp::Add, Box::new(off), Box::new(this_size)),
                });
                let sym_base = name.trim_start_matches('.').replace('.', "_");
                out.push(SectionsItem::Output(def));
                out.push(SectionsItem::Assign(Assignment {
                    symbol: format!("__load_start_{}", sym_base),
                    op: AssignOp::Set, expr: Expr::LoadAddrOf(name.clone()),
                    provide: true, hidden: false,
                }));
                out.push(SectionsItem::Assign(Assignment {
                    symbol: format!("__load_stop_{}", sym_base),
                    op: AssignOp::Set,
                    expr: Expr::Bin(BinOp::Add,
                        Box::new(Expr::LoadAddrOf(name.clone())),
                        Box::new(Expr::SizeOf(name.clone()))),
                    provide: true, hidden: false,
                }));
            }
            t => return Err(format!("unexpected token in OVERLAY body: {:?}", t)),
        }
    }
    // Trailing `>region` / `:phdr` / `= fill` for the overlay as a whole.
    loop {
        let save = lx.save();
        match lx.next() {
            Tok::Punct(">") | Tok::Punct(":") => { let _ = lx.next(); }
            Tok::Punct("=") => { let _ = parse_expr(lx)?; }
            _ => { lx.restore(save); break; }
        }
    }
    Ok(())
}

fn parse_output_section_body(lx: &mut Lexer, def: &mut OutputSecDef)
    -> Result<(), String>
{
    loop {
        let save = lx.save();
        match lx.next() {
            Tok::Punct("}") => break,
            Tok::Eof => return Err("unterminated output section".into()),
            Tok::Ident(kw) if kw == "KEEP" => {
                expect(lx, "(")?;
                let mut spec = parse_input_spec(lx)?;
                spec.keep = true;
                expect(lx, ")")?;
                def.items.push(SecItem::Input(spec));
            }
            Tok::Ident(kw) if kw == "ASSERT" => {
                let (e, msg) = parse_assert_args(lx)?;
                eat_semi(lx);
                def.items.push(SecItem::Assert(e, msg));
            }
            Tok::Ident(kw) if kw == "PROVIDE" || kw == "PROVIDE_HIDDEN" || kw == "HIDDEN" => {
                expect(lx, "(")?;
                let a = parse_assignment_inner(lx, kw != "HIDDEN", kw != "PROVIDE")?;
                expect(lx, ")")?;
                eat_semi(lx);
                def.items.push(SecItem::Assign(a));
            }
            Tok::Ident(kw) if kw == "CONSTRUCTORS" => {
                def.items.push(SecItem::Constructors);
            }
            Tok::Ident(kw) if kw == "FILL" => {
                expect(lx, "(")?;
                let e = parse_expr(lx)?;
                expect(lx, ")")?;
                eat_semi(lx);
                def.fill = eval_const(&e);
            }
            Tok::Ident(kw) if kw == "BYTE" || kw == "SHORT" || kw == "LONG" || kw == "QUAD" => {
                // These directives consume space *and* write the expression in
                // target byte order.  Merely skipping them used to omit the
                // Linux setup image's 0x5a5aaa55 signature and left every
                // following location-counter assignment four bytes early.
                let width = match kw.as_str() {
                    "BYTE" => 1,
                    "SHORT" => 2,
                    "LONG" => 4,
                    "QUAD" => 8,
                    _ => unreachable!(),
                };
                expect(lx, "(")?;
                let expr = parse_expr(lx)?;
                expect(lx, ")")?;
                eat_semi(lx);
                def.items.push(SecItem::Data { width, expr });
            }
            Tok::Ident(_) => {
                // assignment or file-pattern input spec like *(...) — but '*' is
                // lexed as Punct, so an Ident here is an assignment.
                lx.restore(save);
                if let Some(a) = try_parse_assignment(lx)? {
                    def.items.push(SecItem::Assign(a));
                } else {
                    let _ = lx.next(); // give up on this token
                }
            }
            Tok::Punct("*") => {
                // *(patterns)
                lx.restore(save);
                let spec = parse_input_spec(lx)?;
                def.items.push(SecItem::Input(spec));
            }
            Tok::Punct(";") => {}
            t => return Err(format!("unexpected token in section body: {:?}", t)),
        }
    }
    Ok(())
}

fn parse_output_section(lx: &mut Lexer) -> Result<OutputSecDef, String> {
    let name = match lx.next() {
        Tok::Ident(n) => n,
        t => return Err(format!("expected section name, got {:?}", t)),
    };
    let mut def = OutputSecDef {
        name, address: None, info: false, at_lma: None,
        items: Vec::new(), phdrs: Vec::new(), fill: None, align: None,
        region: None, lma_region: None,
    };

    // optional address expression and/or (INFO)/(NOLOAD) before ':'
    loop {
        let save = lx.save();
        match lx.next() {
            Tok::Punct(":") => break,
            Tok::Punct("(") => {
                match lx.next() {
                    Tok::Ident(t) if t == "INFO" || t == "NOLOAD" || t == "COPY" || t == "DSECT" => {
                        def.info = true;
                        expect(lx, ")")?;
                    }
                    _ => {
                        // parenthesized address expression
                        lx.restore(save);
                        def.address = Some(parse_expr(lx)?);
                    }
                }
            }
            Tok::Num(_) | Tok::Ident(_) => {
                lx.restore(save);
                def.address = Some(parse_expr(lx)?);
            }
            t => return Err(format!("unexpected token in section header: {:?}", t)),
        }
    }

    // optional AT(lma) / ALIGN(n) / SUBALIGN(n) after ':'
    loop {
        let save = lx.save();
        match lx.next() {
            Tok::Punct("{") => break,
            Tok::Ident(kw) if kw == "AT" => {
                expect(lx, "(")?;
                def.at_lma = Some(parse_expr(lx)?);
                expect(lx, ")")?;
            }
            Tok::Ident(kw) if kw == "ALIGN" => {
                expect(lx, "(")?;
                def.align = Some(parse_expr(lx)?);
                expect(lx, ")")?;
            }
            Tok::Ident(kw) if kw == "SUBALIGN" => { skip_parens(lx)?; }
            t => return Err(format!("unexpected token before section body: {:?} (pos {})", t, save)),
        }
    }

    parse_output_section_body(lx, &mut def)?;

    // trailing `> region`, `AT> region`, `:phdr(s)` and `= fill`
    loop {
        let save = lx.save();
        match lx.next() {
            Tok::Punct(">") => {
                match lx.next() {
                    Tok::Ident(r) => def.region = Some(r),
                    t => return Err(format!("expected region name after '>', got {:?}", t)),
                }
            }
            // `AT> region` places the LMA in a different region than the VMA:
            // the standard idiom for data that lives in ROM and is copied to
            // RAM at startup.
            Tok::Ident(kw) if kw == "AT" => {
                if lx.next() != Tok::Punct(">") {
                    lx.restore(save);
                    break;
                }
                match lx.next() {
                    Tok::Ident(r) => def.lma_region = Some(r),
                    t => return Err(format!("expected region name after 'AT>', got {:?}", t)),
                }
            }
            Tok::Punct(":") => {
                if let Tok::Ident(ph) = lx.next() { def.phdrs.push(ph); }
            }
            Tok::Punct("=") => {
                let e = parse_expr(lx)?;
                def.fill = eval_const(&e);
            }
            _ => { lx.restore(save); break; }
        }
    }
    Ok(def)
}

/// Parse `*(pat pat ...)` or `SORT(pat)` / `SORT_BY_ALIGNMENT(pat...)` inside
/// a KEEP() or standalone. The leading `*` (file glob) is accepted and ignored
/// (all files match); a leading filename glob other than `*` is also accepted.
fn parse_input_spec(lx: &mut Lexer) -> Result<InputSpec, String> {
    let mut spec = InputSpec { patterns: Vec::new(), keep: false, sort: SortKind::None };
    // consume file glob (usually '*')
    let save = lx.save();
    match lx.next() {
        Tok::Punct("*") => {}
        Tok::Ident(_) => { /* named file pattern; treat as match-all */ }
        _ => { lx.restore(save); }
    }
    expect(lx, "(")?;
    // Inside: raw pattern lexing until ')'
    loop {
        let save2 = lx.save();
        // detect nested SORT / SORT_BY_ALIGNMENT / SORT_BY_INIT_PRIORITY / EXCLUDE_FILE
        match lx.next() {
            Tok::Punct(")") => break,
            Tok::Ident(kw) if kw == "SORT" || kw == "SORT_BY_NAME" => {
                expect(lx, "(")?;
                spec.sort = SortKind::ByName;
                while let Some(p) = lx.next_pattern() { spec.patterns.push(p); }
                expect(lx, ")")?;
            }
            Tok::Ident(kw) if kw == "SORT_BY_ALIGNMENT" => {
                expect(lx, "(")?;
                spec.sort = SortKind::ByAlignment;
                while let Some(p) = lx.next_pattern() { spec.patterns.push(p); }
                expect(lx, ")")?;
            }
            Tok::Ident(kw) if kw == "SORT_BY_INIT_PRIORITY" => {
                expect(lx, "(")?;
                spec.sort = SortKind::ByInitPriority;
                while let Some(p) = lx.next_pattern() { spec.patterns.push(p); }
                expect(lx, ")")?;
            }
            Tok::Ident(kw) if kw == "EXCLUDE_FILE" => { skip_parens(lx)?; }
            _ => {
                lx.restore(save2);
                match lx.next_pattern() {
                    Some(p) => spec.patterns.push(p),
                    None => { expect(lx, ")")?; break; }
                }
            }
        }
    }
    Ok(spec)
}

// ── Expression parsing (precedence climbing) ────────────────────────────

fn parse_expr(lx: &mut Lexer) -> Result<Expr, String> {
    parse_ternary(lx)
}

fn parse_ternary(lx: &mut Lexer) -> Result<Expr, String> {
    let c = parse_binary(lx, 0)?;
    let save = lx.save();
    if lx.next() == Tok::Punct("?") {
        let t = parse_expr(lx)?;
        expect(lx, ":")?;
        let f = parse_expr(lx)?;
        return Ok(Expr::Ternary(Box::new(c), Box::new(t), Box::new(f)));
    }
    lx.restore(save);
    Ok(c)
}

fn bin_prec(op: &str) -> Option<(BinOp, u8)> {
    Some(match op {
        "||" => (BinOp::LOr, 1),
        "&&" => (BinOp::LAnd, 2),
        "|" => (BinOp::Or, 3),
        "^" => (BinOp::Xor, 4),
        "&" => (BinOp::And, 5),
        "==" => (BinOp::Eq, 6), "!=" => (BinOp::Ne, 6),
        "<" => (BinOp::Lt, 7), "<=" => (BinOp::Le, 7),
        ">" => (BinOp::Gt, 7), ">=" => (BinOp::Ge, 7),
        "<<" => (BinOp::Shl, 8), ">>" => (BinOp::Shr, 8),
        "+" => (BinOp::Add, 9), "-" => (BinOp::Sub, 9),
        "*" => (BinOp::Mul, 10), "/" => (BinOp::Div, 10), "%" => (BinOp::Rem, 10),
        _ => return None,
    })
}

fn parse_binary(lx: &mut Lexer, min_prec: u8) -> Result<Expr, String> {
    let mut lhs = parse_unary(lx)?;
    loop {
        let save = lx.save();
        let tok = lx.next();
        let (op, prec) = match &tok {
            Tok::Punct(p) => match bin_prec(p) {
                Some(x) if x.1 >= min_prec => x,
                _ => { lx.restore(save); return Ok(lhs); }
            },
            _ => { lx.restore(save); return Ok(lhs); }
        };
        let rhs = parse_binary(lx, prec + 1)?;
        lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
    }
}

fn parse_unary(lx: &mut Lexer) -> Result<Expr, String> {
    let save = lx.save();
    match lx.next() {
        Tok::Punct("-") => Ok(Expr::Neg(Box::new(parse_unary(lx)?))),
        Tok::Punct("~") => Ok(Expr::Not(Box::new(parse_unary(lx)?))),
        Tok::Punct("!") => {
            let e = parse_unary(lx)?;
            Ok(Expr::Bin(BinOp::Eq, Box::new(e), Box::new(Expr::Num(0))))
        }
        Tok::Punct("+") => parse_unary(lx),
        _ => { lx.restore(save); parse_primary(lx) }
    }
}

fn parse_primary(lx: &mut Lexer) -> Result<Expr, String> {
    match lx.next() {
        Tok::Num(n) => Ok(Expr::Num(n)),
        Tok::Punct("(") => {
            let e = parse_expr(lx)?;
            expect(lx, ")")?;
            Ok(e)
        }
        Tok::Ident(id) => match id.as_str() {
            "." => Ok(Expr::Dot),
            "ALIGN" => {
                expect(lx, "(")?;
                let a = parse_expr(lx)?;
                let save = lx.save();
                if lx.next() == Tok::Punct(",") {
                    let b = parse_expr(lx)?;
                    expect(lx, ")")?;
                    Ok(Expr::Align2(Box::new(a), Box::new(b)))
                } else {
                    lx.restore(save);
                    expect(lx, ")")?;
                    Ok(Expr::Align1(Box::new(a)))
                }
            }
            "ABSOLUTE" => {
                expect(lx, "(")?;
                let e = parse_expr(lx)?;
                expect(lx, ")")?;
                Ok(Expr::Absolute(Box::new(e)))
            }
            "ADDR" => {
                expect(lx, "(")?;
                let n = ident_arg(lx)?;
                expect(lx, ")")?;
                Ok(Expr::AddrOf(n))
            }
            "LOADADDR" => {
                expect(lx, "(")?;
                let n = ident_arg(lx)?;
                expect(lx, ")")?;
                Ok(Expr::LoadAddrOf(n))
            }
            "SIZEOF" => {
                expect(lx, "(")?;
                let n = ident_arg(lx)?;
                expect(lx, ")")?;
                Ok(Expr::SizeOf(n))
            }
            "ALIGNOF" => {
                expect(lx, "(")?;
                let n = ident_arg(lx)?;
                expect(lx, ")")?;
                Ok(Expr::AlignOf(n))
            }
            "DEFINED" => {
                expect(lx, "(")?;
                let n = ident_arg(lx)?;
                expect(lx, ")")?;
                Ok(Expr::DefinedSym(n))
            }
            // SEGMENT_START("segment", default) -- the value a linker uses for a
            // segment's base, overridable with -Ttext/-Tdata/-Tbss or
            // `-z <seg>=addr`. ld's own default script uses it four times, so
            // any script derived from `ld --verbose` needs it. Without an
            // override the documented behaviour is to yield the default, which
            // is what makes those scripts work unmodified.
            "SEGMENT_START" => {
                expect(lx, "(")?;
                let seg = match lx.next() {
                    Tok::Str(s) => s,
                    t => return Err(format!("SEGMENT_START expects a segment name string, got {:?}", t)),
                };
                expect(lx, ",")?;
                let dflt = parse_expr(lx)?;
                expect(lx, ")")?;
                Ok(Expr::SegmentStart(seg, Box::new(dflt)))
            }
            // DATA_SEGMENT_ALIGN(maxpagesize, commonpagesize) and its END /
            // RELRO_END partners exist so ld can overlap the end of the data
            // segment with the start of the next page. The conservative
            // evaluation -- plain ALIGN to maxpagesize -- is always *correct*;
            // it merely forgoes a few KiB of file-size overlap. Silently
            // rejecting the script, which is what happened before, is not.
            "DATA_SEGMENT_ALIGN" => {
                expect(lx, "(")?;
                let a = parse_expr(lx)?;
                expect(lx, ",")?;
                let _common = parse_expr(lx)?;
                expect(lx, ")")?;
                Ok(Expr::Align1(Box::new(a)))
            }
            "DATA_SEGMENT_RELRO_END" => {
                expect(lx, "(")?;
                let off = parse_expr(lx)?;
                // Second argument is the expression to return.
                let save = lx.save();
                let ret = if lx.next() == Tok::Punct(",") {
                    let e = parse_expr(lx)?; expect(lx, ")")?; e
                } else { lx.restore(save); expect(lx, ")")?; off.clone() };
                Ok(ret)
            }
            "DATA_SEGMENT_END" => {
                expect(lx, "(")?;
                let e = parse_expr(lx)?;
                expect(lx, ")")?;
                Ok(e)
            }
            "MIN" | "MAX" => {
                expect(lx, "(")?;
                let a = parse_expr(lx)?;
                expect(lx, ",")?;
                let b = parse_expr(lx)?;
                expect(lx, ")")?;
                if id == "MIN" { Ok(Expr::Min(Box::new(a), Box::new(b))) }
                else { Ok(Expr::Max(Box::new(a), Box::new(b))) }
            }
            "ASSERT" => {
                expect(lx, "(")?;
                let e = parse_expr(lx)?;
                let mut msg = String::new();
                let save = lx.save();
                if lx.next() == Tok::Punct(",") {
                    if let Tok::Str(s) = lx.next() { msg = s; }
                } else { lx.restore(save); }
                expect(lx, ")")?;
                Ok(Expr::Assert(Box::new(e), msg))
            }
            "SIZEOF_HEADERS" => Ok(Expr::Sym("__SIZEOF_HEADERS".into())),
            "CONSTANT" => {
                expect(lx, "(")?;
                let n = ident_arg(lx)?;
                expect(lx, ")")?;
                Ok(Expr::Num(match n.as_str() {
                    "MAXPAGESIZE" => 0x200000,
                    "COMMONPAGESIZE" => 0x1000,
                    _ => 0,
                }))
            }
            _ => Ok(Expr::Sym(id)),
        },
        t => Err(format!("unexpected token in expression: {:?}", t)),
    }
}

fn ident_arg(lx: &mut Lexer) -> Result<String, String> {
    match lx.next() {
        Tok::Ident(n) => Ok(n),
        t => Err(format!("expected identifier argument, got {:?}", t)),
    }
}

// ── Evaluation ──────────────────────────────────────────────────────────

/// Constant-fold an expression with no symbol/section context (PHDR FLAGS etc).
pub fn eval_const(e: &Expr) -> Option<u64> {
    match e {
        Expr::Num(n) => Some(*n),
        Expr::Neg(x) => eval_const(x).map(|v| v.wrapping_neg()),
        Expr::Not(x) => eval_const(x).map(|v| !v),
        Expr::Bin(op, a, b) => {
            let (a, b) = (eval_const(a)?, eval_const(b)?);
            Some(apply_binop(*op, a, b))
        }
        _ => None,
    }
}

pub fn apply_binop(op: BinOp, a: u64, b: u64) -> u64 {
    match op {
        BinOp::Add => a.wrapping_add(b),
        BinOp::Sub => a.wrapping_sub(b),
        BinOp::Mul => a.wrapping_mul(b),
        BinOp::Div => if b == 0 { 0 } else { a / b },
        BinOp::Rem => if b == 0 { 0 } else { a % b },
        BinOp::Shl => a.wrapping_shl(b as u32),
        BinOp::Shr => a.wrapping_shr(b as u32),
        BinOp::And => a & b,
        BinOp::Or => a | b,
        BinOp::Xor => a ^ b,
        BinOp::Eq => (a == b) as u64,
        BinOp::Ne => (a != b) as u64,
        BinOp::Lt => ((a as i64) < (b as i64)) as u64,
        BinOp::Le => ((a as i64) <= (b as i64)) as u64,
        BinOp::Gt => ((a as i64) > (b as i64)) as u64,
        BinOp::Ge => ((a as i64) >= (b as i64)) as u64,
        BinOp::LAnd => ((a != 0) && (b != 0)) as u64,
        BinOp::LOr => ((a != 0) || (b != 0)) as u64,
    }
}

/// Evaluation context provided by the layout engine.
pub struct EvalCtx<'a> {
    pub dot: u64,
    pub symbols: &'a FxHashMap<String, u64>,
    /// name -> (addr, size, align, lma)
    pub sections: &'a FxHashMap<String, (u64, u64, u64, u64)>,
    /// Segment base overrides from `-Ttext`/`-Tdata`/`-Tbss` and
    /// `-z <segment>=<addr>`, consulted by `SEGMENT_START`. Empty by default,
    /// in which case `SEGMENT_START` yields its declared default.
    pub segment_starts: Option<&'a FxHashMap<String, u64>>,
}

pub enum EvalError {
    UndefinedSymbol(String),
    UnknownSection(String),
    AssertFailed(String),
}

pub fn eval_expr(e: &Expr, ctx: &EvalCtx) -> Result<u64, EvalError> {
    match e {
        Expr::Num(n) => Ok(*n),
        Expr::Dot => Ok(ctx.dot),
        // An explicit -T<seg>/-z override wins; otherwise the script's own
        // default applies, which is what makes `ld --verbose` output usable
        // verbatim.
        Expr::SegmentStart(seg, dflt) => {
            if let Some(map) = ctx.segment_starts {
                if let Some(&v) = map.get(seg.as_str()) {
                    return Ok(v);
                }
            }
            eval_expr(dflt, ctx)
        }
        Expr::Sym(name) => ctx.symbols.get(name).copied()
            .ok_or_else(|| EvalError::UndefinedSymbol(name.clone())),
        Expr::Neg(x) => Ok(eval_expr(x, ctx)?.wrapping_neg()),
        Expr::Not(x) => Ok(!eval_expr(x, ctx)?),
        Expr::Bin(op, a, b) => {
            // Short-circuit logical ops so ASSERT(SIZEOF(.x) == 0 || ...) works
            // even when one side references an empty/missing section.
            match op {
                BinOp::LOr => {
                    let av = eval_expr(a, ctx)?;
                    if av != 0 { return Ok(1); }
                    Ok((eval_expr(b, ctx)? != 0) as u64)
                }
                BinOp::LAnd => {
                    let av = eval_expr(a, ctx)?;
                    if av == 0 { return Ok(0); }
                    Ok((eval_expr(b, ctx)? != 0) as u64)
                }
                _ => Ok(apply_binop(*op, eval_expr(a, ctx)?, eval_expr(b, ctx)?)),
            }
        }
        Expr::Align1(x) => {
            let a = eval_expr(x, ctx)?.max(1);
            Ok((ctx.dot + a - 1) & !(a - 1))
        }
        Expr::Align2(v, a) => {
            let v = eval_expr(v, ctx)?;
            let a = eval_expr(a, ctx)?.max(1);
            Ok((v + a - 1) & !(a - 1))
        }
        Expr::Absolute(x) => eval_expr(x, ctx),
        Expr::AddrOf(name) => ctx.sections.get(name).map(|s| s.0)
            .ok_or_else(|| EvalError::UnknownSection(name.clone())),
        Expr::LoadAddrOf(name) => ctx.sections.get(name).map(|s| s.3)
            .ok_or_else(|| EvalError::UnknownSection(name.clone())),
        Expr::SizeOf(name) => Ok(ctx.sections.get(name).map(|s| s.1).unwrap_or(0)),
        Expr::AlignOf(name) => Ok(ctx.sections.get(name).map(|s| s.2).unwrap_or(1)),
        Expr::DefinedSym(name) => Ok(ctx.symbols.contains_key(name) as u64),
        Expr::Min(a, b) => Ok(eval_expr(a, ctx)?.min(eval_expr(b, ctx)?)),
        Expr::Max(a, b) => Ok(eval_expr(a, ctx)?.max(eval_expr(b, ctx)?)),
        Expr::Assert(x, msg) => {
            let v = eval_expr(x, ctx)?;
            if v == 0 {
                return Err(EvalError::AssertFailed(msg.clone()));
            }
            Ok(v)
        }
        Expr::Ternary(c, t, f) => {
            if eval_expr(c, ctx)? != 0 { eval_expr(t, ctx) } else { eval_expr(f, ctx) }
        }
    }
}

// ── Glob matching for input section patterns ────────────────────────────

/// Match a section name against a linker-script glob (`*`, `?`, `[...]`).
pub fn glob_match(pattern: &str, name: &str) -> bool {
    glob_match_bytes(pattern.as_bytes(), name.as_bytes())
}

fn glob_match_bytes(p: &[u8], n: &[u8]) -> bool {
    // Iterative wildcard matcher with backtracking on '*'.
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star_p, mut star_n) = (usize::MAX, 0usize);
    while ni < n.len() {
        if pi < p.len() {
            match p[pi] {
                b'*' => { star_p = pi; star_n = ni; pi += 1; continue; }
                b'?' => { pi += 1; ni += 1; continue; }
                b'[' => {
                    if let Some((matched, next_pi)) = match_class(p, pi, n[ni]) {
                        if matched { pi = next_pi; ni += 1; continue; }
                    }
                }
                c if c == n[ni] => { pi += 1; ni += 1; continue; }
                _ => {}
            }
        }
        if star_p != usize::MAX {
            // backtrack: let '*' absorb one more character
            pi = star_p + 1;
            star_n += 1;
            ni = star_n;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' { pi += 1; }
    pi == p.len()
}

fn match_class(p: &[u8], start: usize, c: u8) -> Option<(bool, usize)> {
    // p[start] == '['; find closing ']'
    let mut i = start + 1;
    let negate = i < p.len() && (p[i] == b'!' || p[i] == b'^');
    if negate { i += 1; }
    let mut matched = false;
    let class_start = i;
    while i < p.len() && (p[i] != b']' || i == class_start) {
        if i + 2 < p.len() && p[i+1] == b'-' && p[i+2] != b']' {
            if p[i] <= c && c <= p[i+2] { matched = true; }
            i += 3;
        } else {
            if p[i] == c { matched = true; }
            i += 1;
        }
    }
    if i >= p.len() { return None; }
    Some((matched != negate, i + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_basics() {
        assert!(glob_match(".text.*", ".text.foo"));
        assert!(!glob_match(".text.*", ".text"));
        assert!(glob_match(".text", ".text"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match(".text.split.[0-9a-zA-Z_]*", ".text.split.f00"));
        assert!(!glob_match(".text.split.[0-9a-zA-Z_]*", ".text.split.-x"));
        assert!(glob_match("___ksymtab+*", "___ksymtab+foo"));
        assert!(glob_match(".data..hot.*", ".data..hot.x"));
    }

    #[test]
    fn parse_kernel_style_script() {
        let src = r#"
            OUTPUT_FORMAT("elf64-x86-64")
            OUTPUT_ARCH(i386:x86-64)
            ENTRY(phys_startup_64)
            jiffies = jiffies_64;
            PHDRS {
             text PT_LOAD FLAGS(5);
             data PT_LOAD FLAGS(6);
             note PT_NOTE FLAGS(0);
            }
            SECTIONS
            {
             . = (0xffffffff80000000 + (((0x1000000) + (0x200000 - 1)) & ~(0x200000 - 1)));
             phys_startup_64 = ABSOLUTE(startup_64 - 0xffffffff80000000);
             .text : AT(ADDR(.text) - 0xffffffff80000000) {
              _text = .;
              . = ALIGN((1 << 21));
              *(.text .text.fixup)
              . = ALIGN(16); __x_start = .; KEEP(*(SORT(___kentry+*))) __x_end = .;
             } :text = 0xcccccccc
             _etext = .;
             /DISCARD/ : { *(.discard) *(.discard.*) }
             .bss : AT(ADDR(.bss) - 0xffffffff80000000) {
              __bss_start = .;
              *(.bss..page_aligned)
              *(.bss)
              . = ALIGN((1 << 12));
              __bss_stop = .;
             }
             . = ALIGN((1 << 12));
             .brk : { __brk_base = .; . += 64 * 1024; *(.bss..brk) __brk_limit = .; }
             _end = .;
             .got : { *(.got) *(.igot.*) }
             ASSERT(SIZEOF(.got) == 0, "Unexpected GOT entries detected!")
            }
            . = ASSERT((_end - 0xffffffff80000000 <= (512 * 1024 * 1024)), "kernel image bigger than KERNEL_IMAGE_SIZE");
            PROVIDE(__ref_stack_chk_guard = __stack_chk_guard);
        "#;
        let s = parse_linker_script(src).expect("parse failed");
        assert_eq!(s.entry.as_deref(), Some("phys_startup_64"));
        assert_eq!(s.phdrs.len(), 3);
        assert_eq!(s.phdrs[0].name, "text");
        assert_eq!(s.phdrs[0].flags, Some(5));
        let outs: Vec<&OutputSecDef> = s.sections.iter().filter_map(|i| match i {
            SectionsItem::Output(o) => Some(o), _ => None }).collect();
        let names: Vec<&str> = outs.iter().map(|o| o.name.as_str()).collect();
        assert!(names.contains(&".text"));
        assert!(names.contains(&"/DISCARD/"));
        assert!(names.contains(&".brk"));
        let text = outs.iter().find(|o| o.name == ".text").unwrap();
        assert_eq!(text.fill, Some(0xcccccccc));
        assert_eq!(text.phdrs, vec!["text"]);
        assert!(text.at_lma.is_some());
        // dot assignment first
        assert!(matches!(&s.sections[0], SectionsItem::Assign(a) if a.symbol == "."));
    }

    #[test]
    fn parse_phdr_header_membership_and_gnu_types() {
        let script = parse_linker_script(r#"
            PHDRS {
                image PT_LOAD FILEHDR PHDRS FLAGS(5);
                stack PT_GNU_STACK FLAGS(6);
                relro PT_GNU_RELRO FLAGS(4);
            }
            SECTIONS { . = SIZEOF_HEADERS; .text : { *(.text) } :image }
        "#).unwrap();
        assert_eq!(script.phdrs[0].ptype, 1);
        assert!(script.phdrs[0].has_filehdr);
        assert!(script.phdrs[0].has_phdrs);
        assert_eq!(script.phdrs[1].ptype, 0x6474_e551);
        assert_eq!(script.phdrs[1].flags, Some(6));
        assert_eq!(script.phdrs[2].ptype, 0x6474_e552);
    }

    #[test]
    fn expr_eval() {
        let src = "SECTIONS { . = (0x100 + ((0x30 + (0x20 - 1)) & ~(0x20 - 1))); }";
        let s = parse_linker_script(src).unwrap();
        let SectionsItem::Assign(a) = &s.sections[0] else { panic!() };
        let syms = FxHashMap::default();
        let secs = FxHashMap::default();
        let ctx = EvalCtx { dot: 0, symbols: &syms, sections: &secs, segment_starts: None };
        assert_eq!(eval_expr(&a.expr, &ctx).ok(), Some(0x100 + 0x40));
    }

    #[test]
    fn division_and_discard_tokens_do_not_conflict() {
        let src = r#"SECTIONS {
            slots = ((text_size + 255) / 256) + 1;
            /DISCARD/ : { *(.note*) }
        }"#;
        let s = parse_linker_script(src).unwrap();
        let SectionsItem::Assign(a) = &s.sections[0] else { panic!() };
        let mut syms = FxHashMap::default();
        syms.insert("text_size".to_string(), 513);
        let secs = FxHashMap::default();
        let ctx = EvalCtx { dot: 0, symbols: &syms, sections: &secs, segment_starts: None };
        assert_eq!(eval_expr(&a.expr, &ctx).ok(), Some(4));
        assert!(matches!(&s.sections[1],
            SectionsItem::Output(def) if def.name == "/DISCARD/"));
    }

    #[test]
    fn scalar_data_directives_preserve_width_and_expression() {
        let s = parse_linker_script(r#"
            SECTIONS { .signature : {
                BYTE(0x12); SHORT(0x3456) LONG(target - .) QUAD(0x123456789abcdef0)
            } }
        "#).unwrap();
        let SectionsItem::Output(def) = &s.sections[0] else { panic!() };
        let widths: Vec<u8> = def.items.iter().filter_map(|item| match item {
            SecItem::Data { width, .. } => Some(*width),
            _ => None,
        }).collect();
        assert_eq!(widths, vec![1, 2, 4, 8]);
        assert!(matches!(def.items[2], SecItem::Data {
            width: 4, expr: Expr::Bin(BinOp::Sub, _, _)
        }));
    }

    #[test]
    fn malformed_scalar_data_directive_is_rejected() {
        assert!(parse_linker_script(
            "SECTIONS { .x : { LONG(1 +); } }").is_err());
    }

    // ── MEMORY regions ──────────────────────────────────────────────────

    #[test]
    fn memory_regions_parse_with_attributes_and_abbreviations() {
        let s = parse_linker_script(r#"
            MEMORY {
              rom (rx)  : ORIGIN = 0x08000000, LENGTH = 256K
              ram (rwx) : org    = 0x20000000, len    = 64K
              bare      : ORIGIN = 0, LENGTH = 16
            }
            SECTIONS { .text : { *(.text) } > rom }
        "#).unwrap();
        assert_eq!(s.memory.len(), 3);
        assert_eq!(s.memory[0].name, "rom");
        assert_eq!(s.memory[0].attrs, "rx");
        // `org`/`len` are the documented abbreviations and appear in real
        // firmware scripts; rejecting them would be a silent portability wall.
        assert_eq!(s.memory[1].name, "ram");
        assert_eq!(s.memory[2].attrs, "", "attribute list is optional");
        let SectionsItem::Output(def) = &s.sections[0] else { panic!("expected output section") };
        assert_eq!(def.region.as_deref(), Some("rom"));
    }

    #[test]
    fn memory_region_lma_assignment_is_separate_from_vma() {
        // `> ram AT> rom` is the ROM-resident/RAM-executing idiom; the two
        // regions must not be conflated.
        let s = parse_linker_script(r#"
            MEMORY {
              rom (rx)  : ORIGIN = 0x08000000, LENGTH = 256K
              ram (rwx) : ORIGIN = 0x20000000, LENGTH = 64K
            }
            SECTIONS { .data : { *(.data) } > ram AT> rom }
        "#).unwrap();
        let SectionsItem::Output(def) = &s.sections[0] else { panic!() };
        assert_eq!(def.region.as_deref(), Some("ram"));
        assert_eq!(def.lma_region.as_deref(), Some("rom"));
    }

    #[test]
    fn memory_region_without_length_is_rejected() {
        // A region missing ORIGIN/LENGTH cannot be range-checked; accepting it
        // would silently disable the overflow diagnostic.
        assert!(parse_linker_script(
            "MEMORY { r : ORIGIN = 0 } SECTIONS { .text : { *(.text) } }").is_err());
    }

    // ── SEGMENT_START / DATA_SEGMENT_* ──────────────────────────────────

    #[test]
    fn segment_start_uses_default_without_override() {
        let s = parse_linker_script(
            r#"SECTIONS { . = SEGMENT_START("text-segment", 0x400000); }"#).unwrap();
        let SectionsItem::Assign(a) = &s.sections[0] else { panic!() };
        let syms = FxHashMap::default();
        let secs = FxHashMap::default();
        let ctx = EvalCtx { dot: 0, symbols: &syms, sections: &secs, segment_starts: None };
        assert_eq!(eval_expr(&a.expr, &ctx).ok(), Some(0x400000));
    }

    #[test]
    fn segment_start_honours_an_override() {
        let s = parse_linker_script(
            r#"SECTIONS { . = SEGMENT_START("text-segment", 0x400000); }"#).unwrap();
        let SectionsItem::Assign(a) = &s.sections[0] else { panic!() };
        let syms = FxHashMap::default();
        let secs = FxHashMap::default();
        let mut ov = FxHashMap::default();
        ov.insert("text-segment".to_string(), 0xdead000u64);
        let ctx = EvalCtx { dot: 0, symbols: &syms, sections: &secs, segment_starts: Some(&ov) };
        assert_eq!(eval_expr(&a.expr, &ctx).ok(), Some(0xdead000),
                   "-Ttext / -z must be able to override SEGMENT_START");
    }

    #[test]
    fn data_segment_align_is_a_conservative_align() {
        // The overlap optimisation is optional; the alignment is not. Treating
        // DATA_SEGMENT_ALIGN as ALIGN(maxpagesize) is always correct and lets
        // `ld --verbose` output link unmodified.
        let s = parse_linker_script(
            "SECTIONS { . = 0x1001; . = DATA_SEGMENT_ALIGN(0x1000, 0x1000); }").unwrap();
        let SectionsItem::Assign(a) = &s.sections[1] else { panic!() };
        let syms = FxHashMap::default();
        let secs = FxHashMap::default();
        let ctx = EvalCtx { dot: 0x1001, symbols: &syms, sections: &secs, segment_starts: None };
        assert_eq!(eval_expr(&a.expr, &ctx).ok(), Some(0x2000));
    }

    // ── HIDDEN / PROVIDE_HIDDEN ─────────────────────────────────────────

    #[test]
    fn hidden_and_provide_hidden_set_the_hidden_flag() {
        let s = parse_linker_script(r#"
            SECTIONS {
              .text : { *(.text) }
              PROVIDE(vis = .);
              PROVIDE_HIDDEN(ph = .);
              HIDDEN(h = .);
            }
        "#).unwrap();
        let get = |i: usize| -> &Assignment {
            match &s.sections[i] { SectionsItem::Assign(a) => a, _ => panic!("not an assign") }
        };
        assert!(!get(1).hidden && get(1).provide, "PROVIDE: visible, conditional");
        assert!(get(2).hidden && get(2).provide, "PROVIDE_HIDDEN: hidden, conditional");
        // HIDDEN() defines unconditionally -- it is not a PROVIDE.
        assert!(get(3).hidden && !get(3).provide, "HIDDEN: hidden, unconditional");
    }

    // ── OVERLAY ─────────────────────────────────────────────────────────

    #[test]
    fn overlay_members_share_a_vma_and_advance_the_lma() {
        let s = parse_linker_script(r#"
            SECTIONS {
              . = 0x400000;
              .text : { *(.text) }
              OVERLAY 0x500000 : AT (0x480000) {
                .ovl1 { *(.ovl1) }
                .ovl2 { *(.ovl2) }
              }
            }
        "#).unwrap();
        let outs: Vec<&OutputSecDef> = s.sections.iter().filter_map(|i| match i {
            SectionsItem::Output(d) => Some(d), _ => None }).collect();
        let ovl1 = outs.iter().find(|d| d.name == ".ovl1").expect(".ovl1 missing");
        let ovl2 = outs.iter().find(|d| d.name == ".ovl2").expect(".ovl2 missing");
        // Both members are pinned to the overlay's start address...
        assert!(matches!(ovl1.address, Some(Expr::Num(0x500000))));
        assert!(matches!(ovl2.address, Some(Expr::Num(0x500000))));
        // ...while their load addresses differ: the first is the plain base,
        // the second is base + SIZEOF(.ovl1).
        assert!(matches!(ovl1.at_lma, Some(Expr::Num(0x480000))));
        assert!(matches!(ovl2.at_lma, Some(Expr::Bin(BinOp::Add, _, _))),
                "second member's LMA must advance past the first");
    }

    #[test]
    fn overlay_defines_load_start_and_stop_symbols() {
        // The copy routine cannot find the bytes without these, so an overlay
        // that omits them is useless even though it lays out correctly.
        let s = parse_linker_script(r#"
            SECTIONS {
              OVERLAY 0x500000 : AT (0x480000) { .ovl1 { *(.ovl1) } }
            }
        "#).unwrap();
        let names: Vec<&str> = s.sections.iter().filter_map(|i| match i {
            SectionsItem::Assign(a) => Some(a.symbol.as_str()), _ => None }).collect();
        assert!(names.contains(&"__load_start_ovl1"), "got {:?}", names);
        assert!(names.contains(&"__load_stop_ovl1"), "got {:?}", names);
    }
}

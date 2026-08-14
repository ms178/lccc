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

#[derive(Debug, Clone, Default)]
pub struct LinkerScript {
    pub entry: Option<String>,
    pub phdrs: Vec<PhdrDecl>,
    pub sections: Vec<SectionsItem>,
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
            b'/' => !first || c == b'/', // allow /DISCARD/
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

        // identifier / section name / keyword
        if Self::is_ident_char(c, true) {
            let start = self.pos;
            self.pos += 1;
            while self.pos < self.src.len() && Self::is_ident_char(self.src[self.pos], false) {
                self.pos += 1;
            }
            // Special-case /DISCARD/: the trailing slash
            if self.pos < self.src.len() && self.src[self.pos] == b'/' {
                let s = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                if s == "/DISCARD" {
                    self.pos += 1;
                    return Tok::Ident("/DISCARD/".to_string());
                }
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
                "ASSERT" => {
                    let (e, msg) = parse_assert_args(&mut lx)?;
                    script.top_asserts.push((e, msg));
                    eat_semi(&mut lx);
                }
                "PROVIDE" | "PROVIDE_HIDDEN" => {
                    expect(&mut lx, "(")?;
                    let a = parse_assignment_inner(&mut lx, true)?;
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
    Ok(Some(Assignment { symbol: name, op, expr, provide: false }))
}

fn parse_assignment_inner(lx: &mut Lexer, provide: bool) -> Result<Assignment, String> {
    let name = match lx.next() {
        Tok::Ident(n) => n,
        t => return Err(format!("expected symbol in PROVIDE, got {:?}", t)),
    };
    expect(lx, "=")?;
    let expr = parse_expr(lx)?;
    Ok(Assignment { symbol: name, op: AssignOp::Set, expr, provide })
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
            Tok::Ident(kw) if kw == "PROVIDE" || kw == "PROVIDE_HIDDEN" => {
                expect(lx, "(")?;
                let a = parse_assignment_inner(lx, true)?;
                expect(lx, ")")?;
                eat_semi(lx);
                out.push(SectionsItem::Assign(a));
            }
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

fn parse_output_section(lx: &mut Lexer) -> Result<OutputSecDef, String> {
    let name = match lx.next() {
        Tok::Ident(n) => n,
        t => return Err(format!("expected section name, got {:?}", t)),
    };
    let mut def = OutputSecDef {
        name, address: None, info: false, at_lma: None,
        items: Vec::new(), phdrs: Vec::new(), fill: None, align: None,
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

    // section body
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
            Tok::Ident(kw) if kw == "PROVIDE" || kw == "PROVIDE_HIDDEN" => {
                expect(lx, "(")?;
                let a = parse_assignment_inner(lx, true)?;
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
                // data directives - record as an input of literal bytes via assignment hack:
                // represent as ". += N" advance with a fill; rare in kernel scripts (unused).
                skip_parens(lx)?;
                eat_semi(lx);
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

    // trailing :phdr(s) and fill
    loop {
        let save = lx.save();
        match lx.next() {
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
    fn expr_eval() {
        let src = "SECTIONS { . = (0x100 + ((0x30 + (0x20 - 1)) & ~(0x20 - 1))); }";
        let s = parse_linker_script(src).unwrap();
        let SectionsItem::Assign(a) = &s.sections[0] else { panic!() };
        let syms = FxHashMap::default();
        let secs = FxHashMap::default();
        let ctx = EvalCtx { dot: 0, symbols: &syms, sections: &secs };
        assert_eq!(eval_expr(&a.expr, &ctx).ok(), Some(0x100 + 0x40));
    }
}

//! Shared assembler identifier scanning and conservative reference proofs.
//!
//! Builtin-policy opacity uses raw whole-identifier mentions: GAS defines
//! symbols in too many ways for an enumeration to be safe. DCE and inlining
//! share those boundaries but may distinguish a fixed-zero-operand x86
//! instruction from an address reference. That limited proof is disabled
//! across the entire assembly environment when macros/includes are opaque.

/// Is `c` part of an assembler identifier (the base character set the
/// symbol grammar shares with C, plus `.` and `$`)?
#[inline]
pub(crate) fn is_asm_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c == b'$'
}

/// Does the assembler text `asm` name the symbol `name`?
///
/// The match must be a whole identifier: neither preceded nor followed by an
/// identifier character.  `$` needs special care because it is both an
/// identifier character (`foo$bar` is one symbol) and the immediate prefix
/// (`$g` is the address of `g`): a `$` immediately before the match counts as
/// a boundary only when the character before the `$` is not itself an
/// identifier character.
pub(crate) fn asm_mentions_symbol(asm: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let bytes = asm.as_bytes();
    let mut search_from = 0;
    while let Some(rel) = asm[search_from..].find(name) {
        let abs = search_from + rel;
        let before_ok = if abs == 0 {
            true
        } else {
            let prev = bytes[abs - 1];
            if !is_asm_ident_char(prev) {
                true
            } else if prev == b'$' {
                // `$` preceded by non-ident or start => immediate prefix `$g`
                // `$` preceded by ident => embedded `$` like `foo$bar`
                if abs >= 2 {
                    !is_asm_ident_char(bytes[abs - 2])
                } else {
                    true
                }
            } else {
                false
            }
        };
        let after = abs + name.len();
        let after_ok = after == bytes.len() || !is_asm_ident_char(bytes[after]);
        if before_ok && after_ok {
            return true;
        }
        // `name` is a valid UTF-8 substring of `asm`, so this stays on a
        // character boundary (unlike `abs + 1`).
        search_from = abs + name.len();
    }
    false
}

/// Does any of `blobs` mention `name`?
pub(crate) fn any_blob_mentions<S: AsRef<str>>(blobs: &[S], name: &str) -> bool {
    blobs.iter().any(|b| asm_mentions_symbol(b.as_ref(), name))
}

/// Extract the base symbol from an inline-asm operand (`foo+8`, `foo-4`,
/// `foo@GOTPCREL`, `*foo`, `$foo`).
pub(crate) fn asm_symbol_base(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut start = 0;
    while start < bytes.len()
        && !(bytes[start].is_ascii_alphabetic() || bytes[start] == b'_' || bytes[start] == b'.')
    {
        start += 1;
    }
    let mut end = start;
    while end < bytes.len() && is_asm_ident_char(bytes[end]) {
        end += 1;
    }
    if start < end { &s[start..end] } else { s }
}

/// Fixed-zero-operand x86 mnemonics, shared with assembler validation.
///
/// The position in this table doubles as the bit position used by
/// [`AsmSymbolContext::macro_shadowed`], so the two can never drift apart.
const X86_ZERO_OPERAND_MNEMONICS: &[&str] = &[
    "vzeroupper",
    "vzeroall",
    "cpuid",
    "leave",
    "leaveq",
    "hlt",
    "pause",
    "syscall",
    "sysret",
    "ud2",
    "cwtl",
    "cltq",
    "cqto",
    "cltd",
    "cwtd",
    "endbr64",
    "endbr32",
    "cbtw",
    "cbw",
    "cwde",
    "cdqe",
    "cwd",
    "lfence",
    "sfence",
    "mfence",
    "rdtsc",
    "rdtscp",
    "cdq",
    "cqo",
    "stac",
    "clac",
    "clui",
    "stui",
    "serialize",
    "rdpkru",
    "wrpkru",
    "wbnoinvd",
    "clzero",
    "rdpru",
    "mcommit",
];

/// `macro_shadowed` is a `u64`, so the table must stay within 64 entries.
const _: () = assert!(X86_ZERO_OPERAND_MNEMONICS.len() <= 64);

/// Bit position of `mnemonic` in [`X86_ZERO_OPERAND_MNEMONICS`], if it is there.
fn mnemonic_bit(mnemonic: &str) -> Option<u32> {
    X86_ZERO_OPERAND_MNEMONICS
        .iter()
        .position(|m| *m == mnemonic)
        .map(|i| i as u32)
}

/// Fixed-zero-operand x86 mnemonics, shared with assembler validation.
pub(crate) fn is_x86_zero_operand_mnemonic(mnemonic: &str) -> bool {
    mnemonic_bit(mnemonic).is_some()
}

/// Case-insensitive raw-byte substring search.
///
/// The directive scan deliberately does not respect quoting: a blob whose
/// quoting cannot be parsed must not be able to hide a directive from it.
fn contains_ci(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

/// Escapes in quoted data (e.g. EXPORT_SYMBOL's "\\0") do not define
/// macros. Unquoted backslashes can form identifiers or continue statements.
/// Ambiguous/unterminated quoting remains opaque rather than guessing.
fn opaque_asm_quoting(text: &str) -> bool {
    let mut quoted = false;
    let mut escaped = false;
    for byte in text.bytes() {
        if escaped {
            escaped = false;
        } else if quoted && byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            quoted = !quoted;
        } else if !quoted && matches!(byte, b'\\' | b'\'') {
            return true;
        }
    }
    quoted || escaped
}

/// What the whole assembly environment of a translation unit says about a
/// single blob's statement structure.
///
/// Two hazards make the "a standalone known mnemonic is an instruction, not an
/// address of the same-named C symbol" proof unusable, and they have different
/// scopes, so they are tracked separately:
///
/// * A GAS macro defined in one blob can shadow the same-named instruction in
///   every LATER blob (macros persist across inline-asm statements unless
///   purged). That is a per-NAME hazard: only a name some blob actually defines
///   as a macro can be shadowed, so `macro_shadowed` records exactly those.
/// * `.include` can define macros whose names this scanner cannot see, and
///   `.altmacro` changes how GAS parses operands. Either makes every blob's
///   statement structure unknown, so they stay assembly-wide.
///
/// Quoting opacity is NOT assembly-wide and never was a cross-blob hazard: an
/// unparseable quote in one blob says nothing about another blob's statement
/// boundaries, and it cannot hide a `.macro`/`.include` from the raw directive
/// scan above (that scan ignores quoting on purpose). It is therefore evaluated
/// per blob inside [`Self::references_symbol`].
///
/// Lumping it in with the assembly-wide test cost real precision, measured on
/// Linux 6.18.63: `__get_user`/`__put_user`'s exception-table blob
/// (`arch/x86/include/asm/uaccess.h`) both defines `.macro extable_type_reg`
/// and writes `.ifc \reg, %%\rs`, and it was the ONLY opaque blob among the
/// 101 templates of `kernel/futex/core.o`. One blob switched the proof off for
/// the whole TU, so `dead_statics`' inline-asm rooting fell back to raw text
/// mentions and saw the bare mnemonic `stac`/`clac` inside the
/// `.altinstr_replacement` half of every `ALTERNATIVE("", "stac", …)` template.
/// It rooted the two `static __always_inline` helpers of `asm/smap.h`, kept
/// their bodies although nothing calls them (zero relocations against either
/// symbol), and emitted out-of-line copies. objtool validated those dead
/// bodies — its SMAP alternative applies to them too — and reported nine
/// warnings (`return with UACCESS enabled` at `stac+0xb`, `redundant UACCESS
/// disable` at `.altinstr_replacement+0x0`) across `kernel/futex/{core,pi,
/// waitwake}.o`, `arch/x86/kernel/fpu/signal.o` and `lib/iov_iter.o`. gcc-16
/// given the identical recorded command line emits no such symbol at all.
///
/// Default is deliberately conservative (proof off), including in synthetic IR
/// contexts.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AsmSymbolContext {
    /// Target is x86 (`EM_386` / `EM_X86_64`), so the mnemonic table applies.
    x86: bool,
    /// `.include` or `.altmacro` in some blob, or a macro directive whose name
    /// could not be read: no statement proof anywhere.
    globally_opaque: bool,
    /// One bit per [`X86_ZERO_OPERAND_MNEMONICS`] entry, set when some blob
    /// defines (or purges) a GAS macro of that name.
    macro_shadowed: u64,
}

impl AsmSymbolContext {
    pub(crate) fn new<'a>(blobs: impl IntoIterator<Item = &'a str>, machine: u16) -> Self {
        let mut context = Self {
            x86: matches!(machine, 3 | 62),
            globally_opaque: false,
            macro_shadowed: 0,
        };
        if !context.x86 {
            return context;
        }
        for text in blobs {
            let bytes = text.as_bytes();
            if contains_ci(bytes, b".include") || contains_ci(bytes, b".altmacro") {
                context.globally_opaque = true;
                break;
            }
            context.collect_macro_shadows(text);
            if context.globally_opaque {
                break;
            }
        }
        context
    }

    /// Record every mnemonic some `.macro`/`.purgem` in `text` names.
    ///
    /// A name outside the mnemonic table cannot affect the proof — the proof
    /// only ever runs for table entries — so only those need a bit. A directive
    /// whose name cannot be read makes the whole environment opaque rather than
    /// guessing.
    fn collect_macro_shadows(&mut self, text: &str) {
        let bytes = text.as_bytes();
        for directive in [".macro", ".purgem"] {
            let needle = directive.as_bytes();
            let mut search = 0usize;
            while search + needle.len() <= bytes.len() {
                let Some(relative) = bytes[search..]
                    .windows(needle.len())
                    .position(|window| window.eq_ignore_ascii_case(needle))
                else {
                    break;
                };
                let at = search + relative;
                let end = at + needle.len();
                search = end;
                // The directive must be a whole token: `.macrolist` is not
                // `.macro`, and `foo.macro` is not either.
                let starts_token = at == 0 || !is_asm_ident_char(bytes[at - 1]);
                let ends_token = end == bytes.len() || !is_asm_ident_char(bytes[end]);
                if !starts_token || !ends_token {
                    continue;
                }
                // `.macro NAME [args…]` names one macro; `.purgem NAME[, …]`
                // names macros that existed, so both lose the proof for NAME.
                let mut i = end;
                loop {
                    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    let name_start = i;
                    while i < bytes.len() && is_asm_ident_char(bytes[i]) {
                        i += 1;
                    }
                    if i == name_start {
                        self.globally_opaque = true;
                        return;
                    }
                    if let Some(bit) = mnemonic_bit(&text[name_start..i]) {
                        self.macro_shadowed |= 1u64 << bit;
                    }
                    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    if directive == ".purgem" && i < bytes.len() && bytes[i] == b',' {
                        i += 1;
                        continue;
                    }
                    break;
                }
            }
        }
    }

    /// Unlike a raw mention, a standalone known opcode does not take the
    /// same-named C symbol's address. Unknown targets/macros remain opaque.
    pub(crate) fn references_symbol(&self, asm: &str, name: &str) -> bool {
        // The proof needs all of: an x86 target, no `.include`/`.altmacro` and
        // no unreadable macro name anywhere in the environment, a name from the
        // fixed zero-operand mnemonic table, no macro of that name in any blob,
        // and quoting THIS blob that can actually be parsed. Anything else stays
        // a raw mention, which is the conservative answer.
        let proof_available = self.x86
            && !self.globally_opaque
            && mnemonic_bit(name).is_some_and(|bit| self.macro_shadowed >> bit & 1 == 0)
            && !opaque_asm_quoting(asm);
        if !proof_available {
            return asm_mentions_symbol(asm, name);
        }
        // Split only at actual statement boundaries. Preserve quoted text,
        // comments and multiline expressions verbatim; even uncertain mentions
        // remain roots. Only quoted-data escapes are eligible for filtering.
        let bytes = asm.as_bytes();
        let mut start = 0;
        let mut quote = None;
        let mut escaped = false;
        let mut line_comment = false;
        let mut block_comment = false;
        let mut depth = 0usize;
        for (i, &byte) in bytes.iter().enumerate() {
            if block_comment {
                if byte == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    block_comment = false;
                }
                continue;
            }
            if line_comment {
                if byte != b'\n' {
                    continue;
                }
                line_comment = false;
            }
            if let Some(delimiter) = quote {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == delimiter {
                    quote = None;
                }
                continue;
            }
            if byte == b'"' || byte == b'\'' {
                quote = Some(byte);
            } else if byte == b'#' || (byte == b'/' && bytes.get(i + 1) == Some(&b'/')) {
                line_comment = true;
            } else if byte == b'/' && bytes.get(i + 1) == Some(&b'*') {
                block_comment = true;
            } else if matches!(byte, b'(' | b'[' | b'{') {
                depth += 1;
            } else if matches!(byte, b')' | b']' | b'}') {
                depth = depth.saturating_sub(1);
            } else if depth == 0 && matches!(byte, b'\n' | b';') {
                let statement = &asm[start..i];
                if statement.trim() != name && asm_mentions_symbol(statement, name) {
                    return true;
                }
                start = i + 1;
            }
        }
        let statement = &asm[start..];
        statement.trim() != name && asm_mentions_symbol(statement, name)
    }

    pub(crate) fn any_blob_references<S: AsRef<str>>(&self, blobs: &[S], name: &str) -> bool {
        blobs
            .iter()
            .any(|text| self.references_symbol(text.as_ref(), name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn references(text: &str, name: &str, machine: u16) -> bool {
        AsmSymbolContext::new([text], machine).references_symbol(text, name)
    }

    #[test]
    fn standalone_x86_instructions_are_not_symbol_addresses() {
        for machine in [3, 62] {
            for name in ["stac", "clac", "cpuid", "serialize", "vzeroupper"] {
                for text in [
                    name.to_string(),
                    format!(" \t{name}\n"),
                    format!("{name}; {name}\n"),
                ] {
                    assert!(!references(&text, name, machine), "{machine}: {text:?}");
                    assert!(
                        asm_mentions_symbol(&text, name),
                        "opacity stays conservative"
                    );
                }
            }
        }
    }

    #[test]
    fn quoted_export_data_escapes_do_not_hide_instruction_positions() {
        for metadata in [
            r#".ascii "" "\0"; .quad exported"#,
            r#".ascii "escaped\\\";stac;data"; .quad exported"#,
        ] {
            let context = AsmSymbolContext::new([metadata, "stac"], 62);
            assert!(!context.references_symbol("stac", "stac"), "{metadata}");
            assert!(context.references_symbol(".quad stac", "stac"));
        }
        assert!(references(r#".ascii "unterminated; stac"#, "stac", 62));
    }

    #[test]
    fn operands_labels_directives_and_mixed_mentions_survive() {
        for name in ["stac", "clac"] {
            for text in [
                format!("call {name}"),
                format!(".quad {name}"),
                format!("{name}:"),
                format!(".set alias, {name}+8"),
                format!("{name}; .quad ${name}"),
                format!("movq {name}@GOTPCREL(%rip), %rax"),
                format!(".quad ({name})"),
                format!(".quad (\n{name}\n)"),
                format!(r#".quad "{name}""#),
                format!(r#".ascii "x;{name};y""#),
                format!("1: {name}"),
                format!("# {name}\n"),
                format!("/*\n{name}\n*/"),
                format!("{name} # comment"),
            ] {
                assert!(references(&text, name, 62), "{text:?}");
            }
        }
    }

    #[test]
    fn non_x86_and_unknown_names_remain_opaque() {
        for machine in [0, 40, 183, 243] {
            assert!(references("stac; clac", "stac", machine));
            assert!(references("stac; clac", "clac", machine));
        }
        assert!(references("unknown", "unknown", 62));
        assert!(!references("stac_extra; xstac; .text.stac", "stac", 62));
        assert!(!references("stac", "", 62));
        assert!(AsmSymbolContext::default().references_symbol("stac", "stac"));
    }

    #[test]
    fn assembly_wide_opacity_sources() {
        // `.include` can define macros this scanner cannot name, `.altmacro`
        // changes operand parsing, and a macro directive whose name cannot be
        // read leaves the shadow set unknown. All three are assembly-wide.
        for opaque in [
            r#".include "other.inc""#,
            ".INCLUDE \"other.inc\"",
            ".altmacro",
            ".macro",
            ".macro\n",
        ] {
            for machine in [3, 62] {
                let context = AsmSymbolContext::new([opaque, "stac"], machine);
                assert!(context.references_symbol("stac", "stac"), "{opaque}");
            }
        }
        // Quoting that cannot be parsed makes THAT blob opaque, per blob.
        assert!(references(".quad \\\nstac", "stac", 62));
        assert!(references("'stac", "stac", 62));
    }

    #[test]
    fn macro_shadowing_is_scoped_to_the_shadowed_name() {
        // A macro named like the mnemonic can shadow the instruction in every
        // later blob, so the proof is off for exactly that name. The directive
        // spelling is case-insensitive; the NAME is not (GAS semantics), so only
        // an exact-case match shadows.
        for definition in [
            ".macro stac",
            ".MACRO stac",
            ".macro stac flags:req",
            ".purgem stac",
        ] {
            for machine in [3, 62] {
                let context = AsmSymbolContext::new([definition, "stac"], machine);
                assert!(context.references_symbol("stac", "stac"), "{definition}");
            }
        }
        // A different-case name is a different macro and shadows nothing.
        let context = AsmSymbolContext::new([".macro STAC", "stac"], 62);
        assert!(!context.references_symbol("stac", "stac"));
        // A macro of a DIFFERENT name shadows nothing the proof cares about, and
        // neither does a blob whose quoting cannot be parsed: quoting is a
        // per-blob property, not an assembly-wide one.
        for unrelated in [
            ".macro op",
            ".MACRO op",
            ".purgem op",
            "\\argument",
            ".macro op x=1",
        ] {
            for machine in [3, 62] {
                let context = AsmSymbolContext::new([unrelated, "stac"], machine);
                assert!(!context.references_symbol("stac", "stac"), "{unrelated}");
            }
        }
    }

    #[test]
    fn kernel_alternative_template_is_not_a_stac_clac_reference() {
        // Linux 6.18.63 `arch/x86/include/asm/smap.h`, verbatim shape of the
        // `alternative("", "stac", X86_FEATURE_SMAP)` expansion. The mnemonic
        // appears only as a standalone statement inside `.altinstr_replacement`,
        // so it is the instruction, not the address of the `static
        // __always_inline void stac(void)` it is spelled like. Rooting it made
        // `dead_statics` keep both helpers, lccc emitted out-of-line bodies
        // nothing calls, and objtool reported nine UACCESS warnings on the dead
        // copies; gcc-16 emits no such symbol.
        for (name, template) in [
            (
                "stac",
                "# ALT: oldinstr\n771:\n\t\n772:\n# ALT: padding\n.skip -(((775f-774f)-(772b-771b)) > 0) * ((775f-774f)-(772b-771b)),0x90\n773:\n.pushsection .altinstructions,\"a\"\n .long 771b - .\n .long 774f - .\n .4byte ( 9*32+20)\n .byte 773b-771b\n .byte 775f-774f\n.popsection\n.pushsection .altinstr_replacement, \"ax\"\n# ALT: replacement\n774:\n\tstac\n775:\n.popsection\n",
            ),
            (
                "clac",
                "# ALT: oldinstr\n771:\n\t\n772:\n# ALT: padding\n.skip -(((775f-774f)-(772b-771b)) > 0) * ((775f-774f)-(772b-771b)),0x90\n773:\n.pushsection .altinstructions,\"a\"\n .long 771b - .\n .long 774f - .\n .4byte ( 9*32+20)\n .byte 773b-771b\n .byte 775f-774f\n.popsection\n.pushsection .altinstr_replacement, \"ax\"\n# ALT: replacement\n774:\n\tclac\n775:\n.popsection\n",
            ),
        ] {
            assert!(
                !opaque_asm_quoting(template),
                "{name}: fixture must be parseable"
            );
            // The blob that made the whole environment opaque in the kernel TU:
            // `__get_user`/`__put_user`'s exception-table entry, which defines
            // `.macro extable_type_reg` and writes `.ifc \reg, %%\rs`.
            let uaccess_blob = "1:\tlock cmpxchgl %3, %2\n2:\n .pushsection \"__ex_table\",\"a\"\n .balign 4\n .long (1b) - .\n .long (2b) - .\n.macro extable_type_reg type:req reg:req\n.set .Lfound, 0\n.irp rs,rax,rcx,rdx\n.ifc \\reg, %%\\rs\n.set .Lfound, 1\n.endif\n.endr\n.endm\nextable_type_reg reg=%0, type=(17 | ((-14) << 16)) \n.purgem extable_type_reg\n .popsection\n";
            for blobs in [
                vec![template],
                vec![uaccess_blob, template],
                vec![template, uaccess_blob],
            ] {
                let context = AsmSymbolContext::new(blobs.iter().copied(), 62);
                assert!(
                    !context.references_symbol(template, name),
                    "{name}: {} blobs",
                    blobs.len()
                );
                assert!(
                    !context.any_blob_references(&blobs, name),
                    "{name}: any_blob_references"
                );
            }
            // A genuine address reference in the same environment still roots.
            let context = AsmSymbolContext::new([uaccess_blob, ".quad stac"], 62);
            assert!(context.references_symbol(".quad stac", "stac"));
        }
    }

    #[test]
    fn whole_identifiers_only() {
        assert!(asm_mentions_symbol(".globl memset", "memset"));
        assert!(asm_mentions_symbol("memset:", "memset"));
        assert!(asm_mentions_symbol("call memcpy@PLT", "memcpy"));
        assert!(!asm_mentions_symbol("memcpyx:", "memcpy"));
        assert!(!asm_mentions_symbol("xmemcpy:", "memcpy"));
        assert!(!asm_mentions_symbol("", "memcpy"));
        assert!(!asm_mentions_symbol("anything", ""));
    }

    #[test]
    fn address_forms_are_mentions() {
        // Inline-asm operand syntax: the `%`-escaped form of an address.
        assert!(asm_mentions_symbol("movl g(%%rip), %0", "g"));
        assert!(asm_mentions_symbol("movl g@GOTPCREL(%%rip), %0", "g"));
        assert!(!asm_mentions_symbol("movl other(%%rip), %0", "g"));
        assert!(!asm_mentions_symbol("movl foobar, %0", "foo"));
        assert!(!asm_mentions_symbol("movl foo, %0", "foobar"));
        assert!(!asm_mentions_symbol("movl g_long, %0", "g"));
        assert!(!asm_mentions_symbol("movl my_g, %0", "g"));
    }

    #[test]
    fn immediate_prefix_and_embedded_dollar() {
        assert!(asm_mentions_symbol("movabsq $g, %rax", "g"), "$g immediate");
        assert!(asm_mentions_symbol("movl $g+4, %eax", "g"), "$g+4");
        assert!(asm_mentions_symbol("leaq $g, %rax", "g"), "leaq $g");
        assert!(asm_mentions_symbol("$g", "g"), "bare $g");
        // An embedded `$` makes `foo$bar` one identifier: neither half of it
        // may be reported as a mention.
        assert!(!asm_mentions_symbol("movl foo$bar, %0", "bar"));
        assert!(!asm_mentions_symbol("movl foo$bar, %0", "foo"));
    }

    #[test]
    fn dollar_is_both_an_identifier_char_and_an_immediate_prefix() {
        // `$g` is an immediate reference to `g`.
        assert!(asm_mentions_symbol("movq $g, %rax", "g"));
        // `foo$g` is a single symbol name, not a reference to `g`.
        assert!(!asm_mentions_symbol("foo$g:", "g"));
    }

    #[test]
    fn any_blob_mentions_is_the_disjunction() {
        let blobs = vec![".text".to_string(), "memcpy:".to_string()];
        assert!(any_blob_mentions(&blobs, "memcpy"));
        assert!(!any_blob_mentions(&blobs, "memset"));
    }
}

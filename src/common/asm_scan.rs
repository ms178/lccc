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
pub(crate) fn is_x86_zero_operand_mnemonic(mnemonic: &str) -> bool {
    matches!(
        mnemonic,
        "vzeroupper"
            | "vzeroall"
            | "cpuid"
            | "leave"
            | "leaveq"
            | "hlt"
            | "pause"
            | "syscall"
            | "sysret"
            | "ud2"
            | "cwtl"
            | "cltq"
            | "cqto"
            | "cltd"
            | "cwtd"
            | "endbr64"
            | "endbr32"
            | "cbtw"
            | "cbw"
            | "cwde"
            | "cdqe"
            | "cwd"
            | "lfence"
            | "sfence"
            | "mfence"
            | "rdtsc"
            | "rdtscp"
            | "cdq"
            | "cqo"
            | "stac"
            | "clac"
            | "clui"
            | "stui"
            | "serialize"
            | "rdpkru"
            | "wrpkru"
            | "wbnoinvd"
            | "clzero"
            | "rdpru"
            | "mcommit"
    )
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

/// Assembly-wide facts: a macro in one blob can override an opcode in another.
/// Default is deliberately conservative, including in synthetic IR contexts.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AsmSymbolContext {
    distinguish_x86_opcodes: bool,
}

impl AsmSymbolContext {
    pub(crate) fn new<'a>(blobs: impl IntoIterator<Item = &'a str>, machine: u16) -> Self {
        let distinguish_x86_opcodes = matches!(machine, 3 | 62)
            && !blobs.into_iter().any(|text| {
                opaque_asm_quoting(text)
                    || [".macro", ".include", ".altmacro", ".purgem"]
                        .iter()
                        .any(|directive| {
                            text.as_bytes()
                                .windows(directive.len())
                                .any(|part| part.eq_ignore_ascii_case(directive.as_bytes()))
                        })
            });
        Self {
            distinguish_x86_opcodes,
        }
    }

    /// Unlike a raw mention, a standalone known opcode does not take the
    /// same-named C symbol's address. Unknown targets/macros remain opaque.
    pub(crate) fn references_symbol(&self, asm: &str, name: &str) -> bool {
        if !self.distinguish_x86_opcodes || !is_x86_zero_operand_mnemonic(name) {
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
    fn opaque_context_in_another_blob_disables_the_proof() {
        for opaque in [
            ".macro op",
            ".MACRO op",
            r#".include "other.inc""#,
            ".altmacro",
            ".purgem op",
            "\\argument",
        ] {
            for machine in [3, 62] {
                let context = AsmSymbolContext::new([opaque, "stac"], machine);
                assert!(context.references_symbol("stac", "stac"), "{opaque}");
            }
        }
        assert!(references(".quad \\\nstac", "stac", 62));
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

//! Word-boundary scanning of assembler text for symbol mentions.
//!
//! Two consumers need the same question answered — "does this assembler
//! text name this symbol?" — from opposite sides of the tree:
//!
//! * `passes::dead_statics` uses it to keep a symbol whose address the TU
//!   references only from top-level `asm("...")`.
//! * `common::builtin` (the A13/A14 symbol inventory) uses it to refuse a
//!   synthesised library call when the same TU's top-level assembly defines
//!   or redirects that symbol: the compiler cannot assume the library's
//!   semantics for a name the user has redefined in assembly.
//!
//! Keeping one implementation means the two cannot disagree about what
//! counts as a mention, and a miss is a shared, testable fact rather than
//! two independent guesses.

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

#[cfg(test)]
mod tests {
    use super::*;

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

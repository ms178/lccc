//! Preprocessor conditional expression evaluation.
//!
//! This module handles evaluation of preprocessor conditional expressions (`#if`, `#elif`),
//! including `defined()` operator resolution, `__has_builtin()`, `__has_attribute()`,
//! `__has_include()`, and `__has_include_next()` detection,
//! and replacing undefined identifiers with 0 per the C standard.
//!
//! All scanning operates on byte slices for performance (no `Vec<char>` allocation).

use super::pipeline::Preprocessor;
use super::utils::{
    bytes_to_str, is_digit_separator, is_ident_cont_byte, is_ident_start_byte, push_char_at,
    skip_literal_bytes,
};

impl Preprocessor {
    /// Replace `defined(X)`, `defined X`, `__has_builtin(X)`, `__has_attribute(X)`,
    /// `__has_feature(X)`, `__has_extension(X)`, `__has_include(X)`, and
    /// `__has_include_next(X)` with 0 or 1 in a #if expression.
    /// Replace `__has_builtin(X)`, `__has_attribute(X)`, `__has_feature(X)`
    /// and `__has_extension(X)` with 0/1 in an ORDINARY code line.
    ///
    /// GCC and Clang implement these as preprocessor built-in macros, so they
    /// expand EVERYWHERE — not just inside #if. The kernel relies on that:
    /// kernel/trace/trace.c computes `10*1000 + 4*100 + __has_attribute(
    /// btf_type_tag)` in a plain C expression. Note `defined` and
    /// `__has_include` are NOT handled here: `defined` is only an operator
    /// inside #if/#elif (a code-line `defined` is an ordinary identifier),
    /// and __has_include's argument grammar (<...>) is directive-only.
    /// `diag_line` is the 1-BASED source line of the code line being
    /// expanded (same convention as `process_directive`): any diagnostic
    /// produced here (`__has_warning` contract violations) must point at
    /// the line the programmer wrote.
    pub(super) fn resolve_has_macros_in_code(&mut self, line: &str, diag_line: usize) -> String {
        // Fast path: the overwhelming majority of lines have no __has_ token.
        if !line.contains("__has_") {
            return line.to_string();
        }
        let mut result = String::with_capacity(64);
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            // Skip string and char literals verbatim: `"__has_attribute"` in
            // a string must not be rewritten.
            if bytes[i] == b'"' || (bytes[i] == b'\'' && !is_digit_separator(bytes, i)) {
                let end = skip_literal_bytes(bytes, i, bytes[i]).min(len);
                result.push_str(&line[i..end]);
                i = end;
                continue;
            }
            if is_ident_start_byte(bytes[i]) {
                let start = i;
                i += 1;
                while i < len && is_ident_cont_byte(bytes[i]) {
                    i += 1;
                }
                let ident = bytes_to_str(bytes, start, i);
                // A user macro of the same spelling was already expanded by
                // the caller; a suppressed operator is now an ordinary name.
                if super::macro_defs::is_query_operator(ident)
                    && !self.macros.query_operator_active(ident)
                {
                    result.push_str(ident);
                    continue;
                }
                match ident {
                    "__has_builtin" => {
                        let val = self.resolve_has_builtin_call_bytes(bytes, &mut i, diag_line);
                        result.push_str(val);
                    }
                    "__has_attribute" => {
                        let val =
                            self.resolve_has_attribute_call_bytes(bytes, &mut i, false, diag_line);
                        result.push_str(val);
                    }
                    "__has_c_attribute" => {
                        let val =
                            self.resolve_has_attribute_call_bytes(bytes, &mut i, true, diag_line);
                        result.push_str(val);
                    }
                    "__has_feature" => {
                        let val =
                            self.resolve_has_feature_call_bytes(bytes, &mut i, false, diag_line);
                        result.push_str(val);
                    }
                    "__has_extension" => {
                        let val =
                            self.resolve_has_feature_call_bytes(bytes, &mut i, true, diag_line);
                        result.push_str(val);
                    }
                    "__has_warning" => {
                        let val = self.resolve_has_warning_call_bytes(bytes, &mut i, diag_line);
                        result.push_str(&val);
                    }
                    _ => result.push_str(ident),
                }
                continue;
            }
            i = push_char_at(&mut result, line, i);
        }
        result
    }

    /// Does this `-W...` name correspond to a warning this compiler
    /// actually implements (individual flag or accepted group)?
    fn is_recognized_warning_flag(flag: &str) -> bool {
        let Some(rest) = flag.strip_prefix("-W") else {
            return false;
        };
        // -Wno-<name> asks about the same warning as -W<name>.
        let rest = rest.strip_prefix("no-").unwrap_or(rest);
        // The driver and preprocessor must consult the same registry.
        crate::common::error::WarningConfig::recognizes_name(rest)
    }

    /// Clang's `__has_warning("-Wflag")` operator (full contract, Clang
    /// Preprocessor/__has_warning semantics):
    ///   * missing '('           -> error, yields 0
    ///   * zero arguments        -> error (too few arguments), yields 0
    ///   * non-string argument   -> error, yields 0
    ///   * not "-W..."           -> warning (__has_warning expected option
    ///                              name), yields 0
    ///   * unknown -W flag       -> silently yields 0
    ///   * recognized flag       -> yields 1
    /// The verdict consults `is_recognized_warning_flag`, never a blanket
    /// truth value. Adjacent string literals are concatenated; escapes are
    /// honoured; the closing ')' is required.
    fn resolve_has_warning_call_bytes(
        &mut self,
        bytes: &[u8],
        i: &mut usize,
        diag_line: usize,
    ) -> String {
        macro_rules! diag {
            ($msg:expr) => {
                self.errors.push(super::pipeline::PreprocessorDiagnostic {
                    kind: Some(crate::common::error::WarningKind::Cpp),
                    file: self.current_file(),
                    line: diag_line,
                    col: 0,
                    message: $msg.to_string(),
                })
            };
        }
        // Clang-severity split: syntactic violations of the operator are
        // errors; a well-formed call whose argument is not a "-W..."
        // option name is only a WARNING (it must not reject the TU).
        macro_rules! warn {
            ($msg:expr) => {
                self.push_warning(super::pipeline::PreprocessorDiagnostic {
                    kind: Some(crate::common::error::WarningKind::Cpp),
                    file: self.current_file(),
                    line: diag_line,
                    col: 0,
                    message: $msg.to_string(),
                })
            };
        }
        let len = bytes.len();
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        if *i >= len || bytes[*i] != b'(' {
            diag!("missing '(' after '__has_warning'".to_string());
            return "0".to_string();
        }
        *i += 1;
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        if *i < len && bytes[*i] == b')' {
            *i += 1;
            diag!("too few arguments to function call, expected 1, have 0".to_string());
            return "0".to_string();
        }
        if *i >= len || bytes[*i] != b'"' {
            diag!("expected string literal in '__has_warning'".to_string());
            while *i < len && bytes[*i] != b')' {
                *i += 1;
            }
            if *i < len {
                *i += 1;
            } else {
                diag!("missing ')' after '__has_warning'".to_string());
            }
            return "0".to_string();
        }
        // Concatenated string literals, escape-aware.  The argument is
        // collected as bytes: pushing each byte as a `char` would re-encode
        // the bytes of a non-ASCII character one Latin-1 character apiece.
        let mut arg_bytes: Vec<u8> = Vec::new();
        let mut terminated = true;
        while *i < len && bytes[*i] == b'"' {
            *i += 1;
            let mut closed = false;
            while *i < len {
                let c = bytes[*i];
                if c == b'\\' && *i + 1 < len {
                    arg_bytes.extend_from_slice(&bytes[*i..*i + 2]);
                    *i += 2;
                    continue;
                }
                if c == b'"' {
                    closed = true;
                    *i += 1;
                    break;
                }
                arg_bytes.push(c);
                *i += 1;
            }
            if !closed {
                terminated = false;
                break;
            }
            while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
                *i += 1;
            }
        }
        if !terminated {
            diag!("unterminated string literal in '__has_warning'".to_string());
            while *i < len && bytes[*i] != b')' {
                *i += 1;
            }
            if *i < len {
                *i += 1;
            }
            return "0".to_string();
        }
        // Anything but ')' after the literal is malformed.
        let mut junk = false;
        while *i < len && bytes[*i] != b')' {
            if bytes[*i] != b' ' && bytes[*i] != b'\t' {
                junk = true;
            }
            *i += 1;
        }
        if *i < len {
            *i += 1;
        } else {
            diag!("missing ')' after '__has_warning'".to_string());
            return "0".to_string();
        }
        if junk {
            // Malformed tail: treat as not-a-warning without advertising.
            return "0".to_string();
        }
        let arg = String::from_utf8_lossy(&arg_bytes);
        if !arg.starts_with("-W") {
            warn!("__has_warning expected option name".to_string());
            return "0".to_string();
        }
        if Self::is_recognized_warning_flag(&arg) {
            "1".to_string()
        } else {
            "0".to_string()
        }
    }

    pub(super) fn resolve_defined_in_expr(&mut self, expr: &str, diag_line: usize) -> String {
        let mut result = String::with_capacity(64);
        let bytes = expr.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            if is_ident_start_byte(bytes[i]) {
                let start = i;
                i += 1;
                while i < len && is_ident_cont_byte(bytes[i]) {
                    i += 1;
                }
                let ident = bytes_to_str(bytes, start, i);

                // Do not pre-empt a user macro or a suppressed operator.
                // The subsequent macro-expansion pass owns a user definition;
                // a suppressed operator remains an ordinary identifier and the
                // expression parser rejects its stray argument list.
                if super::macro_defs::is_query_operator(ident)
                    && !self.macros.query_operator_active(ident)
                {
                    result.push_str(ident);
                    continue;
                }

                if ident == "defined" {
                    while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
                        i += 1;
                    }
                    let has_paren = if i < len && bytes[i] == b'(' {
                        i += 1;
                        while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
                            i += 1;
                        }
                        true
                    } else {
                        false
                    };

                    let name_start = i;
                    while i < len && is_ident_cont_byte(bytes[i]) {
                        i += 1;
                    }
                    let name = bytes_to_str(bytes, name_start, i);

                    if has_paren {
                        while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
                            i += 1;
                        }
                        if i < len && bytes[i] == b')' {
                            i += 1;
                        }
                    }

                    let is_def = self.macros.is_defined(name);
                    result.push_str(if is_def { "1" } else { "0" });
                } else if ident == "__has_builtin" {
                    let val = self.resolve_has_builtin_call_bytes(bytes, &mut i, diag_line);
                    result.push_str(val);
                } else if ident == "__has_attribute" {
                    let val =
                        self.resolve_has_attribute_call_bytes(bytes, &mut i, false, diag_line);
                    result.push_str(val);
                } else if ident == "__has_c_attribute" {
                    let val = self.resolve_has_attribute_call_bytes(bytes, &mut i, true, diag_line);
                    result.push_str(val);
                } else if ident == "__has_feature" {
                    let val = self.resolve_has_feature_call_bytes(bytes, &mut i, false, diag_line);
                    result.push_str(val);
                } else if ident == "__has_extension" {
                    let val = self.resolve_has_feature_call_bytes(bytes, &mut i, true, diag_line);
                    result.push_str(val);
                } else if ident == "__has_include" {
                    let val = self.resolve_has_include_call_bytes(bytes, &mut i, false);
                    result.push_str(val);
                } else if ident == "__has_include_next" {
                    let val = self.resolve_has_include_call_bytes(bytes, &mut i, true);
                    result.push_str(val);
                } else if ident == "__has_embed" {
                    let val = self.resolve_has_embed_call_bytes(bytes, &mut i, diag_line);
                    result.push_str(&val);
                } else if ident == "__has_warning" {
                    let val = self.resolve_has_warning_call_bytes(bytes, &mut i, diag_line);
                    result.push_str(&val);
                } else {
                    result.push_str(ident);
                }
                continue;
            }

            i = push_char_at(&mut result, expr, i);
        }

        result
    }

    /// Parse `(name)` after `__has_builtin` and return "1" or "0" (byte-oriented).
    /// Malformed operands are GCC-worded errors, not silent zeros; recovery
    /// consumes the balanced argument so no stray token reaches the parser.
    fn resolve_has_builtin_call_bytes(
        &mut self,
        bytes: &[u8],
        i: &mut usize,
        diag_line: usize,
    ) -> &'static str {
        let len = bytes.len();
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        if *i >= len || bytes[*i] != b'(' {
            self.query_diag("missing '(' after '__has_builtin'".to_string(), diag_line);
            // GCC absorbs a following identifier operand in this recovery.
            while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
                *i += 1;
            }
            if *i < len && is_ident_start_byte(bytes[*i]) {
                *i += 1;
                while *i < len && is_ident_cont_byte(bytes[*i]) {
                    *i += 1;
                }
            }
            return "0";
        }
        *i += 1;
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        if *i >= len || !is_ident_start_byte(bytes[*i]) {
            self.skip_open_paren_arg_bytes(bytes, i);
            self.query_diag(
                "macro '__has_builtin' requires an identifier".to_string(),
                diag_line,
            );
            return "0";
        }
        let start = *i;
        while *i < len && is_ident_cont_byte(bytes[*i]) {
            *i += 1;
        }
        let raw = bytes_to_str(bytes, start, *i);
        // GCC expands the operand before judging the token sequence; a macro
        // expanding to several tokens diagnoses the FIRST expanded token.
        let expanded = self.expand_query_identifier(raw);
        let first = match &expanded {
            Ok(name) => name.as_ref(),
            Err(text) => text.split_whitespace().next().unwrap_or(raw),
        };
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        if *i >= len || bytes[*i] != b')' {
            self.skip_open_paren_arg_bytes(bytes, i);
            self.query_diag(format!("expected ')' after '{first}'"), diag_line);
            return "0";
        }
        *i += 1;
        let name = match expanded {
            Ok(name) => name,
            Err(_) => {
                self.query_diag(format!("expected ')' after '{first}'"), diag_line);
                return "0";
            }
        };
        // GCC 16.2 also answers 1 for the unprefixed library spelling
        // (`__has_builtin(memcpy)` == `__has_builtin(__builtin_memcpy)`).
        // Canonicalize through LCCC's own registry so the claim stays
        // truthful: a name LCCC does not implement still answers 0.
        let name = Self::canonical_builtin_name(&name);
        // GCC 16.2 conditions many GNU x86 builtins on a target flag
        // (measured: `__has_builtin(__builtin_ia32_maxps256)` is 0 at
        // default and 1 under -mavx, while the call itself already errors
        // without the extension). The availability table mirrors that
        // sweep; the feature macros it names are defined by exactly the
        // same flags GCC uses to answer, implication chain included.
        if Self::is_supported_builtin(&name)
            && crate::common::x86_builtin_availability::x86_builtin_feature_macro(&name)
                .is_none_or(|feature_macro| self.macros.is_defined(feature_macro))
        {
            "1"
        } else {
            "0"
        }
    }

    /// Parse `(name)` after `__has_feature` / `__has_extension` and answer
    /// from the capability table in the driver-selected mode.
    ///
    /// GCC 16.2 macro-expands the operand before interpreting it (Clang does
    /// not), so `#define FEAT c_atomic` makes `__has_feature(FEAT)` answer 1.
    /// A malformed operand is an error and yields 0; recovery consumes the
    /// whole balanced operand so no stray token reaches the expression parser.
    fn resolve_has_feature_call_bytes(
        &mut self,
        bytes: &[u8],
        i: &mut usize,
        extension: bool,
        diag_line: usize,
    ) -> &'static str {
        let op = if extension {
            "__has_extension"
        } else {
            "__has_feature"
        };
        let len = bytes.len();
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        if *i >= len || bytes[*i] != b'(' {
            self.query_diag(format!("missing '(' after '{op}'"), diag_line);
            return "0";
        }
        *i += 1;
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        let start = *i;
        if *i >= len || !is_ident_start_byte(bytes[*i]) {
            self.skip_open_paren_arg_bytes(bytes, i);
            self.query_diag(format!("macro '{op}' requires an identifier"), diag_line);
            return "0";
        }
        while *i < len && is_ident_cont_byte(bytes[*i]) {
            *i += 1;
        }
        let raw = bytes_to_str(bytes, start, *i);
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        if *i >= len || bytes[*i] != b')' {
            self.skip_open_paren_arg_bytes(bytes, i);
            self.query_diag(format!("expected ')' after '{raw}'"), diag_line);
            return "0";
        }
        *i += 1;
        let name = match self.expand_query_identifier(raw) {
            Ok(name) => name,
            // The operand expanded to something that is not one identifier.
            // GCC names the FIRST EXPANDED token, not the macro spelling
            // (measured: `#define NOTIDENT c_atomic tls` diagnoses 'c_atomic'),
            // which is also what the code-line path already prints because it
            // expands the whole line first.  The two paths must agree.
            Err(expanded) => {
                let first = expanded.split_whitespace().next().unwrap_or(raw);
                self.query_diag(format!("expected ')' after '{first}'"), diag_line);
                return "0";
            }
        };
        if super::feature_queries::resolve(&name, extension, self.feature_mode) {
            "1"
        } else {
            "0"
        }
    }

    /// Push a GCC-worded error for a malformed `__has_*` operand.  These are
    /// constraint violations: GCC and Clang both reject the translation unit.
    fn query_diag(&mut self, message: String, diag_line: usize) {
        self.errors.push(super::pipeline::PreprocessorDiagnostic {
            kind: Some(crate::common::error::WarningKind::Cpp),
            file: self.current_file(),
            line: diag_line,
            col: 0,
            message,
        });
    }

    /// Macro-expand a `__has_*` identifier operand the way GCC 16.2 does.
    ///
    /// The code-line path expands the whole line before resolving operators,
    /// so this helper is what makes `#if __has_feature(FEAT)` agree with
    /// `int x = __has_feature(FEAT);`.  `Err` carries the expansion that was
    /// not a single identifier so the caller can quote GCC's token in its
    /// diagnostic.  A non-macro operand is returned borrowed: the common case
    /// allocates nothing.
    fn expand_query_identifier<'a>(
        &self,
        name: &'a str,
    ) -> Result<std::borrow::Cow<'a, str>, String> {
        if self.macros.get(name).is_none() {
            return if is_query_identifier(name) {
                Ok(std::borrow::Cow::Borrowed(name))
            } else {
                Err(name.to_string())
            };
        }
        let expanded = self.macros.expand_line(name);
        let trimmed = expanded.trim();
        if !is_query_identifier(trimmed) {
            return Err(expanded);
        }
        Ok(std::borrow::Cow::Owned(trimmed.to_string()))
    }

    /// Parse `(attribute-token)` after `__has_attribute` (`c_syntax` false)
    /// or `__has_c_attribute` (true) and return its value, following GCC
    /// 16.2 in every dialect:
    /// * a standard attribute (C23 6.7.13), in either spelling, is
    ///   `202311` under both operators;
    /// * `gnu::name` / `__gnu__::__name__` is 1 when `name` is a supported
    ///   GNU attribute, and any other vendor scope is 0;
    /// * an unscoped GNU attribute is 1 under `__has_attribute` but 0 under
    ///   `__has_c_attribute` (the `[[...]]` syntax spells it `gnu::name`).
    ///
    /// GCC macro-expands the scope and name identifiers (measured:
    /// `#define SCOPE gnu` / `#define NAME deprecated` both answer 1), and an
    /// unscoped macro may expand to a whole scoped token. Malformed operands
    /// are errors; GCC's diagnostic spelling is `__has_attribute` for BOTH
    /// operators (measured on gcc-16.2, including `__has_c_attribute()`).
    fn resolve_has_attribute_call_bytes(
        &mut self,
        bytes: &[u8],
        i: &mut usize,
        c_syntax: bool,
        diag_line: usize,
    ) -> &'static str {
        let len = bytes.len();
        let skip_blanks = |i: &mut usize| {
            while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
                *i += 1;
            }
        };
        let ident = |i: &mut usize| {
            let start = *i;
            while *i < len && is_ident_cont_byte(bytes[*i]) {
                *i += 1;
            }
            bytes_to_str(bytes, start, *i)
        };
        skip_blanks(i);
        if *i >= len || bytes[*i] != b'(' {
            self.query_diag("missing '(' after '__has_attribute'".to_string(), diag_line);
            skip_blanks(i);
            if *i < len && is_ident_start_byte(bytes[*i]) {
                ident(i);
            }
            return "0";
        }
        *i += 1;
        skip_blanks(i);
        if *i >= len || !is_ident_start_byte(bytes[*i]) {
            self.skip_open_paren_arg_bytes(bytes, i);
            self.query_diag(
                "macro '__has_attribute' requires an identifier".to_string(),
                diag_line,
            );
            return "0";
        }
        let first = ident(i);
        skip_blanks(i);
        let scoped = *i + 1 < len && bytes[*i] == b':' && bytes[*i + 1] == b':';
        let (scope, name) = if scoped {
            *i += 2;
            skip_blanks(i);
            if *i >= len || !is_ident_start_byte(bytes[*i]) {
                self.skip_open_paren_arg_bytes(bytes, i);
                self.query_diag(
                    "attribute identifier required after scope".to_string(),
                    diag_line,
                );
                return "0";
            }
            (Some(first), ident(i))
        } else {
            (None, first)
        };
        skip_blanks(i);
        if *i >= len || bytes[*i] != b')' {
            self.skip_open_paren_arg_bytes(bytes, i);
            self.query_diag("missing ')' after '__has_attribute'".to_string(), diag_line);
            return "0";
        }
        *i += 1;
        // Expand the identifiers the way GCC does. A failed expansion is not
        // an attribute token; answer 0 rather than advertising a capability.
        let (scope, name) = if let Some(scope_raw) = scope {
            let Ok(s) = self.expand_query_identifier(scope_raw) else {
                return "0";
            };
            let Ok(n) = self.expand_query_identifier(name) else {
                return "0";
            };
            (Some(s.into_owned()), n.into_owned())
        } else {
            // An unscoped operand may expand to a WHOLE scoped token
            // (`#define SCOPED gnu::deprecated`), so this cannot use the
            // single-identifier query helper; re-split the expansion.
            let expanded = if self.macros.get(name).is_some() {
                self.macros.expand_line(name)
            } else {
                name.to_string()
            };
            let text = expanded.trim();
            match text.split_once("::") {
                Some((sc, n))
                    if is_query_identifier(sc.trim()) && is_query_identifier(n.trim()) =>
                {
                    (Some(sc.trim().to_string()), n.trim().to_string())
                }
                _ if is_query_identifier(text) => (None, text.to_string()),
                _ => return "0",
            }
        };
        attribute_token_value(scope.as_deref(), &name, c_syntax)
    }

    /// Parse `(<header.h>)` or `("header.h")` after `__has_include` / `__has_include_next`
    /// and return "1" or "0" based on whether the header can be found.
    fn resolve_has_include_call_bytes(
        &mut self,
        bytes: &[u8],
        i: &mut usize,
        is_next: bool,
    ) -> &'static str {
        let len = bytes.len();
        // Skip whitespace
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        if *i >= len || bytes[*i] != b'(' {
            return "0";
        }
        *i += 1; // skip '('
        // Skip whitespace
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }

        // Determine if system include (<...>) or quoted ("...")
        let (header_name, is_system) = if *i < len && bytes[*i] == b'<' {
            *i += 1; // skip '<'
            let start = *i;
            while *i < len && bytes[*i] != b'>' {
                *i += 1;
            }
            let name = bytes_to_str(bytes, start, *i);
            if *i < len {
                *i += 1;
            } // skip '>'
            (name, true)
        } else if *i < len && bytes[*i] == b'"' {
            *i += 1; // skip '"'
            let start = *i;
            while *i < len && bytes[*i] != b'"' {
                *i += 1;
            }
            let name = bytes_to_str(bytes, start, *i);
            if *i < len {
                *i += 1;
            } // skip closing '"'
            (name, false)
        } else {
            // Fallback: try to read as identifier (e.g. macro-expanded argument)
            let start = *i;
            while *i < len && bytes[*i] != b')' {
                *i += 1;
            }
            let name = bytes_to_str(bytes, start, *i).trim();
            (name, false)
        };

        // Skip to closing paren
        while *i < len && bytes[*i] != b')' {
            *i += 1;
        }
        if *i < len {
            *i += 1;
        } // skip ')'

        if header_name.is_empty() {
            return "0";
        }

        // Use the preprocessor's include path resolution
        let found = if is_next {
            self.resolve_include_next_path(header_name).is_some()
        } else {
            self.resolve_include_path(header_name, is_system).is_some()
        };

        if found { "1" } else { "0" }
    }

    /// C23 `__has_embed(...)`: parse the balanced parenthesized argument
    /// (filename spec + optional embed parameters) and probe the resource.
    /// Yields the numeric `__STDC_EMBED_*` value: 0 not found / unsupported
    /// parameter, 1 found and non-empty, 2 found but empty.
    fn resolve_has_embed_call_bytes(
        &mut self,
        bytes: &[u8],
        i: &mut usize,
        diag_line: usize,
    ) -> String {
        let len = bytes.len();
        while *i < len && (bytes[*i] == b' ' || bytes[*i] == b'\t') {
            *i += 1;
        }
        if *i >= len || bytes[*i] != b'(' {
            // Malformed operand; the #if evaluator treats it as absent.
            return "0".to_string();
        }
        *i += 1; // skip '('
        let start = *i;
        let mut depth = 1usize;
        // Literal-aware balance scan: parens inside string/char literals
        // do not terminate the operand.
        while *i < len && depth > 0 {
            let c = bytes[*i];
            if c == b'"' || c == b'\'' {
                let quote = c;
                *i += 1;
                while *i < len && bytes[*i] != quote {
                    if bytes[*i] == b'\\' && *i + 1 < len {
                        *i += 1;
                    }
                    *i += 1;
                }
            } else {
                match c {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
            }
            if depth > 0 {
                *i += 1;
            }
        }
        let inner = bytes_to_str(bytes, start, *i);
        if *i < len {
            *i += 1; // skip ')'
        }
        (self.probe_embed(inner, diag_line) as i64).to_string()
    }

    /// Consume the remainder of an already-opened, malformed `__has_*`
    /// operand through its *matching* ')' (byte-oriented).  The opening '('
    /// has already been consumed by the caller; a missing ')' consumes to
    /// EOL.  Nesting is tracked and string/char literals are skipped whole so
    /// recovery cannot leave a stray ')' behind or let a quote swallow the
    /// rest of the line.  The loop is bounded by the slice length: `depth`
    /// only decrements on ')' and returns at zero, so it cannot underflow.
    fn skip_open_paren_arg_bytes(&self, bytes: &[u8], i: &mut usize) {
        let len = bytes.len();
        let mut depth = 1usize;
        while *i < len {
            match bytes[*i] {
                b'"' | b'\'' => {
                    *i = skip_literal_bytes(bytes, *i, bytes[*i]).min(len);
                    continue;
                }
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        *i += 1;
                        return;
                    }
                }
                _ => {}
            }
            *i += 1;
        }
    }

    /// Map an unprefixed library builtin spelling to its `__builtin_` form
    /// when (and only when) LCCC implements that builtin.  Already-prefixed
    /// and directly-registered names are returned borrowed: no allocation on
    /// the common path.
    fn canonical_builtin_name<'a>(name: &'a str) -> std::borrow::Cow<'a, str> {
        if name.starts_with("__builtin_") || crate::frontend::sema::builtins::is_builtin(name) {
            return std::borrow::Cow::Borrowed(name);
        }
        let prefixed = format!("__builtin_{name}");
        if crate::frontend::sema::builtins::is_builtin(&prefixed) {
            std::borrow::Cow::Owned(prefixed)
        } else {
            std::borrow::Cow::Borrowed(name)
        }
    }

    /// Check if a builtin function name is supported by this compiler.
    /// Delegates to the sema builtin registry (BUILTIN_MAP + atomic builtins)
    /// and adds a few names handled as special syntax rather than function calls.
    pub(super) fn is_supported_builtin(name: &str) -> bool {
        // Check the canonical builtin registry (covers BUILTIN_MAP, __builtin_choose_expr,
        // __builtin_unreachable, __builtin_trap, and all __atomic_*/__sync_* builtins)
        if crate::frontend::sema::builtins::is_builtin(name) {
            return true;
        }
        // These are handled as special syntax (dedicated AST nodes or parser keywords)
        // rather than through the normal builtin function call path, so they are not
        // in the sema builtin registry but are still supported.
        matches!(
            name,
            "__builtin_va_arg" |              // Special token (BuiltinVaArg)
            "__builtin_types_compatible_p" |  // Special AST node (BuiltinTypesCompatibleP)
            "__builtin_convertvector" |       // Special AST node (ConvertVector)
            "__builtin_bit_cast" |            // Special AST node (BitCast)
            "__builtin_offsetof" |            // Predefined macro
            "__builtin_LINE" |                // Expanded by the preprocessor
            "__builtin_FILE" |
            "__builtin_FUNCTION"
        )
    }

    /// Check if an attribute is supported by this compiler.
    pub(super) fn is_supported_attribute(name: &str) -> bool {
        matches!(
            name,
            "aligned"
                | "__aligned__"
                | "packed"
                | "__packed__"
                | "unused"
                | "__unused__"
                | "used"
                | "__used__"
                | "weak"
                | "__weak__"
                | "alias"
                | "__alias__"
                | "ifunc"
                | "__ifunc__"
                | "counted_by"
                | "__counted_by__"
                | "section"
                | "__section__"
                | "visibility"
                | "__visibility__"
                | "deprecated"
                | "__deprecated__"
                | "noreturn"
                | "__noreturn__"
                | "noinline"
                | "__noinline__"
                | "always_inline"
                | "__always_inline__"
                | "stack_protect"
                | "__stack_protect__"
                | "no_stack_protector"
                | "__no_stack_protector__"
                | "diagnose_as"
                | "__diagnose_as"
                | "__diagnose_as__"
                | "diagnose_as_builtin"
                | "__diagnose_as_builtin__"
                | "constructor"
                | "__constructor__"
                | "destructor"
                | "__destructor__"
                | "format"
                | "__format__"
                | "warn_unused_result"
                | "__warn_unused_result__"
                | "nonnull"
                | "__nonnull__"
                | "const"
                | "__const__"
                | "pure"
                | "__pure__"
                | "cold"
                | "__cold__"
                | "hot"
                | "__hot__"
                | "malloc"
                | "__malloc__"
                | "sentinel"
                | "__sentinel__"
                | "may_alias"
                | "__may_alias__"
                | "transparent_union"
                | "__transparent_union__"
                | "error"
                | "__error__"
                | "warning"
                | "__warning__"
                | "cleanup"
                | "__cleanup__"
                | "fallthrough"
                | "__fallthrough__"
                | "flatten"
                | "__flatten__"
                | "nonstring"
                | "__nonstring__"
                | "uninitialized"
                | "__uninitialized__"
                | "annotate"
                | "__annotate__"
                | "no_instrument_function"
                | "__no_instrument_function__"
                | "alloc_size"
                | "__alloc_size__"
                | "format_arg"
                | "__format_arg__"
                | "no_sanitize"
                | "__no_sanitize__"
                | "no_sanitize_address"
                | "__no_sanitize_address__"
                | "no_sanitize_thread"
                | "__no_sanitize_thread__"
                | "no_sanitize_undefined"
                | "__no_sanitize_undefined__"
                | "zero_call_used_regs"
                | "__zero_call_used_regs__"
                | "noclone"
                | "__noclone__"
                | "optimize"
                | "__optimize__"
                | "target"
                | "__target__"
                | "target_clones"
                | "__target_clones__"
                | "assume_aligned"
                | "__assume_aligned__"
                | "returns_nonnull"
                | "__returns_nonnull__"
                | "externally_visible"
                | "__externally_visible__"
                | "artificial"
                | "__artificial__"
                | "leaf"
                | "__leaf__"
                | "access"
                | "__access__"
                | "fd_arg"
                | "__fd_arg__"
                | "tls_model"
                | "__tls_model__"
                // Type attributes the parser implements (parse.rs attribute
                // dispatch) but that were missing here, so feature-tested
                // code (`#if __has_attribute(vector_size)`) silently fell
                // back to its scalar path under lccc while GCC 16 and Clang
                // answer 1 for every one of them (GCC: all but
                // `ext_vector_type`/`address_space`; Clang: all but the
                // GCC-only `mode`/`symver`/`scalar_storage_order`).
                | "vector_size"
                | "__vector_size__"
                | "ext_vector_type"
                | "__ext_vector_type__"
                | "mode"
                | "__mode__"
                | "address_space"
                | "__address_space__"
                | "scalar_storage_order"
                | "__scalar_storage_order__"
                | "common"
                | "__common__"
                | "weakref"
                | "__weakref__"
                | "symver"
                | "__symver__"
                | "gnu_inline"
                | "__gnu_inline__"
                | "fastcall"
                | "__fastcall__"
                | "regparm"
                | "__regparm__"
                | "naked"
                | "__naked__"
        )
    }
}

/// Strip the reserved `__x__` spelling of an attribute name or scope.
fn attribute_base_name(name: &str) -> &str {
    name.strip_prefix("__")
        .and_then(|n| n.strip_suffix("__"))
        .filter(|n| !n.is_empty())
        .unwrap_or(name)
}

/// Is `text` exactly one C identifier (no spaces, operators or punctuation)?
/// `__has_*` operands must be a single identifier after macro expansion.
fn is_query_identifier(text: &str) -> bool {
    let b = text.as_bytes();
    !b.is_empty() && is_ident_start_byte(b[0]) && b.iter().all(|&c| is_ident_cont_byte(c))
}

/// The value of `__has_attribute` / `__has_c_attribute` for one attribute
/// token (see `Preprocessor::resolve_has_attribute_call_bytes`).
fn attribute_token_value(scope: Option<&str>, name: &str, c_syntax: bool) -> &'static str {
    let supported_gnu = |n: &str| Preprocessor::is_supported_attribute(attribute_base_name(n));
    match scope.map(attribute_base_name) {
        Some("gnu") => {
            if supported_gnu(name) {
                "1"
            } else {
                "0"
            }
        }
        Some(_) => "0",
        None => match attribute_base_name(name) {
            "deprecated" | "fallthrough" | "maybe_unused" | "nodiscard" | "noreturn"
            | "_Noreturn" | "unsequenced" | "reproducible" => "202311",
            _ if !c_syntax && supported_gnu(name) => "1",
            _ => "0",
        },
    }
}

#[cfg(test)]
mod has_attribute_tests {
    /// `__has_attribute` / `__has_c_attribute` values, measured on GCC 16.2
    /// (identical under -std=gnu17 and -std=c23).
    #[test]
    fn attribute_token_values_match_gcc() {
        use super::attribute_token_value as v;
        for std_attr in [
            "nodiscard",
            "__nodiscard__",
            "deprecated",
            "fallthrough",
            "maybe_unused",
            "noreturn",
            "_Noreturn",
            "unsequenced",
            "reproducible",
        ] {
            assert_eq!(v(None, std_attr, true), "202311", "{std_attr}");
            assert_eq!(v(None, std_attr, false), "202311", "{std_attr}");
        }
        assert_eq!(v(Some("gnu"), "packed", true), "1");
        assert_eq!(v(Some("__gnu__"), "__packed__", false), "1");
        assert_eq!(v(Some("gnu"), "nonexistent_zz", true), "0");
        assert_eq!(v(Some("clang"), "fallthrough", true), "0");
        assert_eq!(v(None, "packed", true), "0");
        assert_eq!(v(None, "packed", false), "1");
        assert_eq!(v(None, "foo_zz", false), "0");
    }

    use super::*;

    /// Every attribute the parser's attribute dispatch implements must be
    /// reported by `__has_attribute`, otherwise feature-tested code takes
    /// its fallback path under lccc. The list mirrors the named arms in
    /// `parse.rs` (`parse_gnu_attribute_list`); extend both together.
    #[test]
    fn parser_implemented_attributes_are_reported() {
        for name in [
            "aligned",
            "packed",
            "vector_size",
            "ext_vector_type",
            "mode",
            "address_space",
            "scalar_storage_order",
            "common",
            "weakref",
            "ifunc",
            "symver",
            "gnu_inline",
            "fastcall",
            "regparm",
            "naked",
            "cleanup",
            "transparent_union",
            "may_alias",
        ] {
            assert!(
                Preprocessor::is_supported_attribute(name),
                "__has_attribute({name}) must be 1"
            );
            let underscored = format!("__{name}__");
            assert!(
                Preprocessor::is_supported_attribute(&underscored),
                "__has_attribute({underscored}) must be 1"
            );
        }
        assert!(!Preprocessor::is_supported_attribute("no_such_attribute"));
    }
}

#[cfg(test)]
mod has_macro_utf8_tests {
    use super::*;

    #[test]
    fn ordinary_has_macro_expansion_preserves_utf8_literals() {
        let mut pp = Preprocessor::new();
        let source = "const char *s = \"é\"; int c = '__has_feature'; int f = __has_feature(x);";
        let expected = "const char *s = \"é\"; int c = '__has_feature'; int f = 0;";
        assert_eq!(pp.resolve_has_macros_in_code(source, 1), expected);
    }
}

#[cfg(test)]
mod query_operator_suppression_tests {
    use super::*;

    #[test]
    fn suppressed_operator_is_plain_in_code_and_if() {
        let mut pp = Preprocessor::new();
        let out = pp.preprocess(
            "#undef __has_feature\n\
             #ifdef __has_feature\nint defined_yes = 1;\n#else\nint defined_no = 1;\n#endif\n\
             int code = __has_feature(tls);\n\
             #if __has_feature(tls)\nint if_yes = 1;\n#else\nint if_no = 1;\n#endif\n",
        );
        assert!(out.contains("int defined_no = 1;"), "{out}");
        assert!(out.contains("int code = __has_feature(tls);"), "{out}");
        assert!(
            pp.errors()
                .iter()
                .any(|e| e.message.contains("missing binary operator"))
        );
        assert!(
            pp.warnings()
                .iter()
                .any(|w| w.message == "undefining '__has_feature'")
        );
    }

    #[test]
    fn user_macro_wins_in_if_before_expansion() {
        let mut pp = Preprocessor::new();
        let out = pp.preprocess(
            "#define __has_feature(x) 7\n\
             #if __has_feature(tls) == 7\nint yes = 1;\n#else\nint no = 1;\n#endif\n",
        );
        assert!(out.contains("int yes = 1;"), "{out}");
        assert!(!out.contains("int no = 1;"), "{out}");
        assert!(pp.errors().is_empty(), "{:?}", pp.errors());
    }

    #[test]
    fn pop_macro_restores_a_withdrawn_operator() {
        let mut pp = Preprocessor::new();
        let out = pp.preprocess(
            "#pragma push_macro(\"__has_feature\")\n\
             #undef __has_feature\n\
             #pragma pop_macro(\"__has_feature\")\n\
             int code = __has_feature(tls);\n",
        );
        assert!(out.contains("int code = 1;"), "{out}");
    }
}

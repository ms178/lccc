//! Pragma directive handling.
//!
//! Handles #pragma once, pack, push_macro/pop_macro, weak,
//! redefine_extname, GCC visibility and dependency directives, and the
//! message pragmas (`GCC warning`, `GCC error`, `message`).

use std::path::Path;

use super::pipeline::{PragmaDiagnostic, PragmaSeverity, Preprocessor, PreprocessorDiagnostic};
use super::utils::is_ident_cont_byte;

/// `text` after a leading `keyword` token, or `None` when `text` does not
/// start with that whole token (`warning` must not match `warnings`).
pub(super) fn strip_pragma_keyword<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(keyword)?;
    if rest.bytes().next().is_some_and(is_ident_cont_byte) {
        return None;
    }
    Some(rest.trim_start())
}

/// The byte length of the narrow string literal `"..."` that `text` starts
/// with, closing quote included, or `None` when `text` does not start with a
/// complete one (no opening quote, or a quote escaped up to the end).
fn string_literal_len(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// The value of the string literal `text` starts with, and the text after it.
fn leading_string_literal(text: &str) -> Option<(Vec<u8>, &str)> {
    let len = string_literal_len(text)?;
    let value = crate::frontend::lexer::Lexer::narrow_string_literal(&text[..len])?;
    Some((value, &text[len..]))
}

/// Outcome of parsing a `#pragma message` operand (GCC 16.2's
/// `handle_pragma_message`, measured case by case against `gcc-16`).
#[derive(Debug, PartialEq, Eq)]
enum MessageOperand {
    /// The concatenated string, and whether tokens followed it.
    Message { text: Vec<u8>, junk: bool },
    /// No string where one is required: "expected a string after ...".
    MissingString,
    /// A parenthesised string not followed by `)`: "malformed ... ignored".
    Malformed,
}

/// Parse the (already macro-expanded) operand of `#pragma message`: one or
/// more adjacent narrow string literals, optionally inside one pair of
/// parentheses.  Adjacent literals concatenate, as translation phase 6
/// would join them.
fn parse_message_operand(operand: &str) -> MessageOperand {
    fn strings(mut rest: &str) -> Option<(Vec<u8>, &str)> {
        let (mut text, after) = leading_string_literal(rest.trim_start())?;
        rest = after;
        while let Some((more, after)) = leading_string_literal(rest.trim_start()) {
            text.extend_from_slice(&more);
            rest = after;
        }
        Some((text, rest))
    }
    let operand = operand.trim_start();
    let (text, rest) = if let Some(inner) = operand.strip_prefix('(') {
        let Some((text, rest)) = strings(inner) else {
            return MessageOperand::MissingString;
        };
        let Some(rest) = rest.trim_start().strip_prefix(')') else {
            return MessageOperand::Malformed;
        };
        (text, rest)
    } else {
        let Some(found) = strings(operand) else {
            return MessageOperand::MissingString;
        };
        found
    };
    MessageOperand::Message {
        text,
        junk: !rest.trim().is_empty(),
    }
}

impl Preprocessor {
    /// Record a file as protected by `#pragma once`.
    pub(super) fn record_pragma_once(&mut self, path: &Path) {
        self.pragma_once_files.insert(path.to_path_buf());
        if let Some(id) = Self::file_identity(path) {
            self.pragma_once_identities.insert(id);
        }
    }

    /// Return true if a resolved include path has previously used `#pragma once`.
    pub(super) fn is_pragma_once_file(&self, path: &Path) -> bool {
        self.pragma_once_files.contains(path)
            || Self::file_identity(path).is_some_and(|id| self.pragma_once_identities.contains(&id))
    }

    /// Linux file identity for robust #pragma once tracking.
    fn file_identity(path: &Path) -> Option<(u64, u64)> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            None
        }
    }

    pub(super) fn handle_pragma(
        &mut self,
        rest: &str,
        line_num: usize,
        col: usize,
    ) -> Option<String> {
        let rest = rest.trim();
        if let Some(message) = crate::common::diagnostic_pragma::threshold_error(rest) {
            self.errors.push(super::pipeline::PreprocessorDiagnostic {
                kind: crate::common::error::WarningKind::Cpp,
                file: self.current_file(),
                line: line_num,
                col,
                message,
            });
            return None;
        }

        if self.diagnostic_pragmas.apply(rest) {
            // Keep the directive at its exact output position for parser/sema.
            return Some(format!("#pragma {rest}\n"));
        }
        if rest.starts_with("STDC") && rest.split_whitespace().eq(["STDC", "FENV_ACCESS", "ON"]) {
            self.fenv_access_requested = true;
            return None;
        }
        if rest == "once" {
            // Mark the current file as "include once".  Track both canonical
            // path and device/inode so the same file included through a symlink
            // or hardlink is still skipped, matching GCC/Clang behavior.
            if let Some(current_file) = self.include_stack.last().cloned() {
                self.record_pragma_once(&current_file);
            }
            return None;
        }

        // Handle #pragma pack directives (suppress in asm mode since these
        // emit synthetic __ccc_pack_* tokens that the assembler can't parse)
        if let Some(pack_content) = rest.strip_prefix("pack") {
            if self.macros.asm_mode {
                return None;
            }
            return self.handle_pragma_pack(pack_content.trim());
        }

        // Handle #pragma push_macro("name") / pop_macro("name")
        if let Some(push_content) = rest.strip_prefix("push_macro") {
            self.handle_pragma_push_macro(push_content.trim());
            return None;
        }
        if let Some(pop_content) = rest.strip_prefix("pop_macro") {
            self.handle_pragma_pop_macro(pop_content.trim());
            return None;
        }

        // Handle #pragma weak symbol [= alias]
        if let Some(weak_content) = rest.strip_prefix("weak") {
            self.handle_pragma_weak(weak_content.trim());
            return None;
        }

        // Handle #pragma redefine_extname old new
        if let Some(redefine_content) = rest.strip_prefix("redefine_extname") {
            self.handle_pragma_redefine_extname(redefine_content.trim());
            return None;
        }

        // Handle #pragma GCC visibility push(hidden|default|protected|internal) / pop
        // Suppressed in asm mode: synthetic __ccc_visibility_* tokens are C parser-
        // specific and would cause assembler errors when preprocessing .S files.
        if let Some(gcc_content) = rest.strip_prefix("GCC") {
            let gcc_content = gcc_content.trim();
            if let Some(vis_content) = gcc_content.strip_prefix("visibility") {
                if self.macros.asm_mode {
                    return None;
                }
                return self.handle_pragma_gcc_visibility(vis_content.trim());
            }
            if let Some(dep_content) = gcc_content.strip_prefix("dependency") {
                self.handle_pragma_gcc_dependency(dep_content.trim(), line_num, col);
                return None;
            }
            for (keyword, is_error) in [("warning", false), ("error", true)] {
                if let Some(operand) = strip_pragma_keyword(gcc_content, keyword) {
                    self.handle_pragma_gcc_message(operand, keyword, is_error, line_num, col);
                    return None;
                }
            }
        }

        // `#pragma message "..."` / `#pragma message ("...")`.
        if let Some(operand) = strip_pragma_keyword(rest, "message") {
            let expanded = self.macros.expand_line(operand);
            // Assembly mode expands the operand, but neither handles the
            // C-frontend message nor preserves its directive (GCC 16.2).
            if self.macros.asm_mode {
                return None;
            }
            self.handle_pragma_message(&expanded, line_num, col);
            // Preserve the frontend directive in -E output. The lexer ignores
            // it on direct compilations; collected diagnostics own reporting.
            return Some(format!("#pragma message {expanded}\n"));
        }

        // Other pragmas (GCC, diagnostic, etc.) are silently ignored
        None
    }

    /// Handle `#pragma GCC dependency FILENAME [MESSAGE]` (GCC extension).
    ///
    /// GCC semantics, which Clang's corpus pins: the filename is NOT
    /// macro-expanded (a bare macro name is an error) and must be a
    /// complete `"..."` or `<...>` token; the file is searched like a
    /// quoted/chevron include and a missing file is a hard error. When the
    /// dependency exists but is NEWER than the current source file, GCC
    /// warns (regeneration needed), appending any trailing message text.
    /// The dependency is checked only, never inserted.
    fn handle_pragma_gcc_dependency(&mut self, content: &str, line_num: usize, col: usize) {
        let (filename, is_system, rest_after) = if let Some(r) = content.strip_prefix('"') {
            match r.find('"') {
                Some(end) => (r[..end].to_string(), false, &r[end + 1..]),
                None => {
                    self.errors.push(super::pipeline::PreprocessorDiagnostic {
                        kind: crate::common::error::WarningKind::Cpp,
                        file: self.current_file(),
                        line: line_num,
                        col,
                        message: "expected \"FILENAME\" or <FILENAME>".to_string(),
                    });
                    return;
                }
            }
        } else if let Some(r) = content.strip_prefix('<') {
            match r.find('>') {
                Some(end) => (r[..end].to_string(), true, &r[end + 1..]),
                None => {
                    self.errors.push(super::pipeline::PreprocessorDiagnostic {
                        kind: crate::common::error::WarningKind::Cpp,
                        file: self.current_file(),
                        line: line_num,
                        col,
                        message: "expected \"FILENAME\" or <FILENAME>".to_string(),
                    });
                    return;
                }
            }
        } else {
            self.errors.push(super::pipeline::PreprocessorDiagnostic {
                kind: crate::common::error::WarningKind::Cpp,
                file: self.current_file(),
                line: line_num,
                col,
                message: "expected \"FILENAME\" or <FILENAME>".to_string(),
            });
            return;
        };
        let Some(resolved) = self.resolve_include_path(&filename, is_system) else {
            self.errors.push(super::pipeline::PreprocessorDiagnostic {
                kind: crate::common::error::WarningKind::Cpp,
                file: self.current_file(),
                line: line_num,
                col,
                message: format!("'{}' file not found", filename),
            });
            return;
        };
        // Staleness check: warn when the dependency is newer than the
        // current source (its generated content may be out of date). Any
        // trailing text after the filename is GCC's optional message.
        let message = rest_after.trim();
        let dep_newer = match (
            std::fs::metadata(&resolved).and_then(|m| m.modified()),
            std::fs::metadata(self.current_file()).and_then(|m| m.modified()),
        ) {
            (Ok(dep), Ok(cur)) => dep > cur,
            _ => false,
        };
        if dep_newer {
            let text = if message.is_empty() {
                format!("\"{}\" is newer than the current file", filename)
            } else {
                format!(
                    "\"{}\" is newer than the current file: {}",
                    filename, message
                )
            };
            self.push_warning(super::pipeline::PreprocessorDiagnostic {
                kind: crate::common::error::WarningKind::Cpp,
                file: self.current_file(),
                line: line_num,
                col,
                message: text,
            });
        }
    }

    /// `#pragma GCC warning "text"` / `#pragma GCC error "text"` (libcpp's
    /// `do_pragma_warning_or_error`): the operand is ONE narrow string
    /// literal, not macro-expanded and not parenthesised, and its value is
    /// the whole message.  Anything else -- a macro name, `("text")`, no
    /// operand, a prefixed literal -- is an ERROR for both spellings (GCC
    /// 16.2: "invalid '#pragma GCC warning' directive").  Tokens after the
    /// literal are ignored, as in GCC.  `GCC error` fails the translation
    /// unit in every mode, `-E` included, exactly like `#error`.
    fn handle_pragma_gcc_message(
        &mut self,
        operand: &str,
        keyword: &str,
        is_error: bool,
        line_num: usize,
        col: usize,
    ) {
        let (message, is_error) = match leading_string_literal(operand) {
            Some((value, _)) => (String::from_utf8_lossy(&value).into_owned(), is_error),
            None => (format!("invalid '#pragma GCC {keyword}' directive"), true),
        };
        let diagnostic = PreprocessorDiagnostic {
            kind: crate::common::error::WarningKind::Pragmas,
            file: self.current_file(),
            line: line_num,
            col,
            message,
        };
        if is_error {
            self.errors.push(diagnostic);
        } else {
            self.pragma_diagnostics.push(PragmaDiagnostic {
                severity: PragmaSeverity::Warning,
                diagnostic,
                warning_config: None,
            });
        }
    }

    /// `#pragma message` (GCC's C front-end handler): the operand IS
    /// macro-expanded, may be parenthesised, and adjacent literals
    /// concatenate.  A well-formed message is a note
    /// `'#pragma message: TEXT'` (none for an empty string); the malformed
    /// shapes are `-Wpragmas` warnings and print nothing else, except junk
    /// after a complete operand, which warns and still prints the note.
    fn handle_pragma_message(&mut self, operand: &str, line_num: usize, col: usize) {
        let file = self.current_file();
        let mut push = |severity: PragmaSeverity, message: String| {
            self.pragma_diagnostics.push(PragmaDiagnostic {
                severity,
                warning_config: matches!(severity, PragmaSeverity::PragmasWarning)
                    .then(|| Box::new(self.diagnostic_pragmas.current.clone())),
                diagnostic: PreprocessorDiagnostic {
                    kind: crate::common::error::WarningKind::Pragmas,
                    file: file.clone(),
                    line: line_num,
                    col,
                    message,
                },
            });
        };
        match parse_message_operand(operand) {
            MessageOperand::Message { text, junk } => {
                if junk {
                    push(
                        PragmaSeverity::PragmasWarning,
                        "junk at end of '#pragma message'".to_string(),
                    );
                }
                if !text.is_empty() {
                    push(
                        PragmaSeverity::Note,
                        format!("'#pragma message: {}'", String::from_utf8_lossy(&text)),
                    );
                }
            }
            MessageOperand::MissingString => push(
                PragmaSeverity::PragmasWarning,
                "expected a string after '#pragma message'".to_string(),
            ),
            MessageOperand::Malformed => push(
                PragmaSeverity::PragmasWarning,
                "malformed '#pragma message', ignored".to_string(),
            ),
        }
    }

    /// Handle #pragma GCC visibility push(hidden|default|protected|internal) / pop.
    /// Emits synthetic tokens for the parser to track default visibility.
    fn handle_pragma_gcc_visibility(&mut self, content: &str) -> Option<String> {
        let content = content.trim();
        if content == "pop" {
            return Some("__ccc_visibility_pop ;\n".to_string());
        }
        if let Some(rest) = content.strip_prefix("push") {
            let rest = rest.trim();
            if rest.starts_with('(') {
                let inner = rest.trim_start_matches('(').trim_end_matches(')').trim();
                match inner {
                    "hidden" | "default" | "protected" | "internal" => {
                        return Some(format!("__ccc_visibility_push_{} ;\n", inner));
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// Handle #pragma push_macro("name") - save the current definition of macro.
    fn handle_pragma_push_macro(&mut self, content: &str) {
        if let Some(name) = Self::extract_pragma_macro_name(content) {
            let saved = self.macros.snapshot_definition(&name);
            self.macro_save_stack.entry(name).or_default().push(saved);
        }
    }

    /// Handle #pragma pop_macro("name") - restore the previously saved definition.
    fn handle_pragma_pop_macro(&mut self, content: &str) {
        if let Some(name) = Self::extract_pragma_macro_name(content) {
            if let Some(stack) = self.macro_save_stack.get_mut(&name) {
                if let Some(saved) = stack.pop() {
                    self.macros.restore_definition(&name, saved);
                }
            }
        }
    }

    /// Extract macro name from pragma argument like ("name").
    fn extract_pragma_macro_name(content: &str) -> Option<String> {
        let content = content.trim();
        if !content.starts_with('(') {
            return None;
        }
        let inner = content.trim_start_matches('(').trim_end_matches(')').trim();
        // Strip quotes
        let name = inner.trim_matches('"');
        if name.is_empty() {
            return None;
        }
        Some(name.to_string())
    }

    /// Handle #pragma weak directives.
    /// Forms:
    ///   #pragma weak symbol         - mark symbol as weak
    ///   #pragma weak symbol = target - symbol becomes a weak alias for target
    fn handle_pragma_weak(&mut self, content: &str) {
        let content = content.trim();
        if content.is_empty() {
            return;
        }
        if let Some(eq_pos) = content.find('=') {
            let symbol = content[..eq_pos].trim().to_string();
            let target = content[eq_pos + 1..].trim().to_string();
            if !symbol.is_empty() && !target.is_empty() {
                self.weak_pragmas.push((symbol, Some(target)));
            }
        } else {
            // Just mark the symbol as weak
            let symbol = content.split_whitespace().next().unwrap_or("").to_string();
            if !symbol.is_empty() {
                self.weak_pragmas.push((symbol, None));
            }
        }
    }

    /// Handle #pragma redefine_extname old new
    /// Redirects external symbol 'old' to 'new' (non-weak alias).
    fn handle_pragma_redefine_extname(&mut self, content: &str) {
        let parts: Vec<&str> = content.split_whitespace().collect();
        if parts.len() >= 2 {
            let old_name = parts[0].to_string();
            let new_name = parts[1].to_string();
            // Redirect external references from old_name to new_name.
            self.redefine_extname_pragmas.push((old_name, new_name));
        }
    }

    /// Handle #pragma pack directives and emit synthetic tokens for the parser.
    /// Supported forms:
    ///   #pragma pack(N)        - set alignment to N
    ///   #pragma pack()         - reset to default alignment
    ///   #pragma pack(push, N)  - push current and set to N
    ///   #pragma pack(push)     - push current (no change)
    ///   #pragma pack(pop)      - restore previous alignment
    fn handle_pragma_pack(&mut self, content: &str) -> Option<String> {
        let content = content.trim();
        // Must start with '('
        if !content.starts_with('(') {
            return None;
        }
        let inner = content.trim_start_matches('(').trim_end_matches(')').trim();

        if inner.is_empty() {
            // #pragma pack() - reset
            return Some("__ccc_pack_reset ;\n".to_string());
        }

        // Check for push/pop
        if inner == "pop" {
            return Some("__ccc_pack_pop ;\n".to_string());
        }

        if let Some(rest) = inner.strip_prefix("push") {
            let rest = rest.trim().trim_start_matches(',').trim();
            if rest.is_empty() {
                // #pragma pack(push) - push current alignment, don't change
                return Some("__ccc_pack_push_only ;\n".to_string());
            }
            // #pragma pack(push, N) - push current and set to N (0 means default)
            if let Ok(n) = rest.parse::<usize>() {
                return Some(format!("__ccc_pack_push_{} ;\n", n));
            }
            return None;
        }

        // #pragma pack(N) - set alignment
        if let Ok(n) = inner.parse::<usize>() {
            return Some(format!("__ccc_pack_set_{} ;\n", n));
        }

        None
    }
}

/// Render the synthetic pragma tokens back into `#pragma` directives for
/// `-E` output.
///
/// The preprocessor translates the two pragmas the C parser cares about
/// into identifiers the lexer turns into dedicated tokens
/// (`__ccc_pack_push_2 ;`). That is the right internal representation, but
/// `-E` is a *product*: its output must be a valid C translation unit that
/// any other compiler, linter or `#include`-harvesting tool can consume, so
/// the markers are translated back on the way out. Each becomes its own
/// line, because a `#pragma` directive may not share a line with code.
///
/// Only the printed form is affected: the tokens the parser consumes are
/// untouched, so `-E` and `-c` agree on the program's meaning.
pub(crate) fn render_pragma_markers(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains("__ccc_") {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 16);
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        let marker_pos = trimmed
            .find("__ccc_pack_")
            .or_else(|| trimmed.find("__ccc_visibility_"));
        let Some(pos) = marker_pos else {
            out.push_str(line);
            continue;
        };
        let (before, marker) = trimmed.split_at(pos);
        let marker = marker.trim_end_matches(|c: char| c == ' ' || c == '\t' || c == ';');
        let directive = if let Some(rest) = marker.strip_prefix("__ccc_pack_") {
            match rest {
                "reset" => Some("#pragma pack()".to_string()),
                "pop" => Some("#pragma pack(pop)".to_string()),
                "push_only" => Some("#pragma pack(push)".to_string()),
                other => other
                    .strip_prefix("push_")
                    .map(|n| format!("#pragma pack(push, {})", n))
                    .or_else(|| {
                        other
                            .strip_prefix("set_")
                            .map(|n| format!("#pragma pack({})", n))
                    }),
            }
        } else {
            marker
                .strip_prefix("__ccc_visibility_")
                .map(|rest| match rest {
                    "pop" => "#pragma GCC visibility pop".to_string(),
                    other => other.strip_prefix("push_").map_or_else(
                        || format!("#pragma GCC visibility {}", rest),
                        |vis| format!("#pragma GCC visibility push({})", vis),
                    ),
                })
        };
        let Some(directive) = directive else {
            out.push_str(line);
            continue;
        };
        out.push_str(before);
        out.push('\n');
        out.push_str(&directive);
        out.push('\n');
    }
    std::borrow::Cow::Owned(out)
}

#[cfg(test)]
mod message_pragma_tests {
    use super::*;

    #[test]
    fn keywords_match_whole_tokens_only() {
        assert_eq!(
            strip_pragma_keyword("warning \"x\"", "warning"),
            Some("\"x\"")
        );
        assert_eq!(
            strip_pragma_keyword("warning\"x\"", "warning"),
            Some("\"x\"")
        );
        assert_eq!(strip_pragma_keyword("warnings \"x\"", "warning"), None);
        assert_eq!(strip_pragma_keyword("warning_x", "warning"), None);
        assert_eq!(strip_pragma_keyword("error", "error"), Some(""));
    }

    #[test]
    fn a_string_literal_ends_at_its_first_unescaped_quote() {
        assert_eq!(string_literal_len("\"ab\" rest"), Some(4));
        assert_eq!(string_literal_len("\"a\\\"b\""), Some(6));
        assert_eq!(string_literal_len("\"a\\\\\""), Some(5));
        // The only closing quote is escaped: unterminated.
        assert_eq!(string_literal_len("\"abc\\\""), None);
        assert_eq!(string_literal_len("\"abc"), None);
        // A prefixed literal is not the narrow string GCC requires.
        assert_eq!(string_literal_len("L\"x\""), None);
        assert_eq!(string_literal_len(""), None);
    }

    #[test]
    fn message_operands_parse_like_gcc() {
        use MessageOperand::{Malformed, Message, MissingString};
        let msg = |text: &str, junk| Message {
            text: text.as_bytes().to_vec(),
            junk,
        };
        assert_eq!(parse_message_operand("\"a\""), msg("a", false));
        assert_eq!(parse_message_operand("(\"a\" \"b\")"), msg("ab", false));
        assert_eq!(parse_message_operand(" ( \"a\" ) "), msg("a", false));
        assert_eq!(parse_message_operand("\"a\" junk"), msg("a", true));
        assert_eq!(parse_message_operand("(\"a\") junk"), msg("a", true));
        assert_eq!(parse_message_operand("\"t\\x41\\n\""), msg("tA\n", false));
        assert_eq!(parse_message_operand("\"\""), msg("", false));
        assert_eq!(parse_message_operand(""), MissingString);
        assert_eq!(parse_message_operand("(1)"), MissingString);
        assert_eq!(parse_message_operand("x"), MissingString);
        assert_eq!(parse_message_operand("(\"a\" 1)"), Malformed);
        assert_eq!(parse_message_operand("(\"a\""), Malformed);
    }

    #[test]
    fn literal_values_are_decoded_by_the_c_lexer() {
        assert_eq!(
            leading_string_literal("\"\\303\\251\" x"),
            Some((vec![0xc3, 0xa9], " x"))
        );
        assert_eq!(
            leading_string_literal("\"\\u00e9\""),
            Some((vec![0xc3, 0xa9], ""))
        );
        assert_eq!(leading_string_literal("'x'"), None);
    }
}

#[cfg(test)]
mod fp_environment_tests {
    use super::*;

    #[test]
    fn active_fp_environment_pragmas_are_monotone_and_tu_local() {
        for (source, expected) in [
            ("int x;", false),
            ("#pragma STDC FENV_ACCESS OFF\nint x;", false),
            ("#if 0\n#pragma STDC FENV_ACCESS ON\n#endif\nint x;", false),
            ("#pragma STDC FENV_ACCESS ON\nint x;", true),
            (
                "#pragma STDC FENV_ACCESS ON\n#pragma STDC FENV_ACCESS OFF\nint x;",
                true,
            ),
            ("_Pragma(\"STDC FENV_ACCESS ON\")\nint x;", true),
        ] {
            let mut pp = Preprocessor::new();
            let _ = pp.preprocess(source);
            assert!(pp.errors().is_empty());
            assert_eq!(pp.fenv_access_requested, expected, "{source}");
        }
        assert!(!Preprocessor::new().fenv_access_requested);
    }
}

#[cfg(test)]
mod pragma_marker_rendering_tests {
    use super::render_pragma_markers;

    #[test]
    fn every_marker_becomes_its_directive_on_its_own_line() {
        let text = "int a; __ccc_pack_push_1 ;\nint b; __ccc_pack_pop ;\n__ccc_pack_set_4 ;\n";
        let rendered = render_pragma_markers(text);
        // A marker line becomes the text that preceded the marker, then the
        // directive at the head of the NEXT line -- the marker and the `;`
        // that followed it disappear, and the line break that ended the
        // marker line is the one that breaks after the residual text. So a
        // marker sitting alone on its line leaves an empty line above its
        // directive, which is exactly the shape `-E` is allowed to show.
        assert_eq!(
            rendered,
            "int a; \n#pragma pack(push, 1)\nint b; \n#pragma pack(pop)\n\n#pragma pack(4)\n"
        );
        // The invariant that matters is ownership, not line count: no text
        // survives in front of a directive that reaches the compiler's
        // `#pragma` handler, so every emitted directive starts its line.
        let directives: Vec<&str> = rendered.lines().filter(|l| l.contains("#pragma")).collect();
        assert_eq!(directives.len(), 3, "{rendered:?}");
        for line in directives {
            assert!(
                line.starts_with("#pragma"),
                "directive shares its line: {line:?}"
            );
        }
    }

    #[test]
    fn push_only_and_reset_keep_their_parentheses() {
        assert_eq!(
            render_pragma_markers("__ccc_pack_push_only ;\n__ccc_pack_reset ;\n"),
            "\n#pragma pack(push)\n\n#pragma pack()\n"
        );
    }

    #[test]
    fn visibility_markers_round_trip() {
        assert_eq!(
            render_pragma_markers("__ccc_visibility_push_hidden ;\n"),
            "\n#pragma GCC visibility push(hidden)\n"
        );
        assert_eq!(
            render_pragma_markers("__ccc_visibility_pop ;\n"),
            "\n#pragma GCC visibility pop\n"
        );
    }

    #[test]
    fn text_without_markers_is_borrowed_untouched() {
        let text = "int main(void) { return 0; }\n";
        let rendered = render_pragma_markers(text);
        assert!(matches!(rendered, std::borrow::Cow::Borrowed(_)));
        assert_eq!(rendered, text);
    }
}

//! GNU `--version-script` parsing, shared by the executable and shared-object
//! emitters.
//!
//! This lived privately inside `emit_shared.rs`, which is why
//! `--version-script` worked for `.so` output and silently did nothing for
//! executables: the executable emitter had no access to it. Sharing the one
//! implementation is what makes the two paths agree by construction rather
//! than by two people remembering to keep them in step — the same lesson as
//! `layout_plan::SegmentPacker`.
//!
//! Scope: the subset GNU ld users actually rely on — a single version node
//! with `global:` / `local:` lists and glob patterns. Symbol *versioning*
//! (multiple nodes, `@`/`@@`) is handled by the shared-object emitter's
//! verdef machinery; this type only answers "is this symbol exported?".

#[derive(Debug, Clone)]
pub struct VersionScript {
    pub version_name: String,
    pub global_patterns: Vec<String>,
    pub local_star: bool,
}

impl VersionScript {
    pub fn parse(path: &str) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        Self::parse_text(&text)
    }

    /// Parse a version node embedded in a full linker script.
    ///
    /// Linux's vDSO keeps `SECTIONS`, `PHDRS`, and `VERSION` in one file rather
    /// than passing a separate `--version-script`. Start at the last VERSION
    /// keyword so braces belonging to SECTIONS cannot be mistaken for the
    /// version node.
    pub fn parse_text(input: &str) -> Option<Self> {
        let text = strip_version_script_comments(input);

        // A *full* linker script with no VERSION node has no version
        // information at all. Without this guard the scan below falls through
        // to the first `{` in the file -- which belongs to SECTIONS or PHDRS --
        // and happily parses `SECTIONS { ... }` as a version node literally
        // named "SECTIONS", exporting a bogus symbol of that name and treating
        // section definitions as symbol patterns. Detect the full-script shape
        // and bail rather than guess.
        let looks_like_full_script = ["SECTIONS", "PHDRS", "MEMORY", "ENTRY", "OUTPUT_FORMAT"]
            .iter()
            .any(|kw| contains_keyword(&text, kw));
        let has_version_kw = contains_keyword(&text, "VERSION");
        if looks_like_full_script && !has_version_kw {
            return None;
        }

        let version_pos = text.rfind("VERSION")
            .filter(|&pos| {
                let before = text[..pos].chars().next_back();
                let after = text[pos + "VERSION".len()..].chars().next();
                !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            });
        let mut text = version_pos.map_or(text.as_str(), |pos| &text[pos + "VERSION".len()..]);
        if version_pos.is_some() {
            // Full scripts spell `VERSION { NODE { global: ...; }; }` while a
            // standalone version script starts directly at `NODE { ... }`.
            // Peel the outer VERSION block and feed the established parser the
            // inner node.
            let outer_open = text.find('{')?;
            text = &text[outer_open + 1..];
        }
        let open = text.find('{')?;
        // An ANONYMOUS version node — `{ global: a; local: *; };` with no name
        // before the brace — is the most common form in the wild (it is what
        // you write when you only want to restrict visibility and do not need
        // symbol versioning). Requiring a name made `parse` return None for
        // exactly those scripts, so the whole script was silently ignored:
        // `local: *;` exported everything. Treat a missing name as the empty
        // version, which is also how GNU ld models it (no verdef is emitted).
        let version_name = text[..open]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();

        let close = text[open + 1..].find('}')? + open + 1;
        let body = &text[open + 1..close];
        let mut global_patterns = Vec::new();
        let mut local_star = false;
        let mut mode: Option<&str> = None;

        for segment in body.split(';') {
            let mut t = segment.trim();
            if t.is_empty() { continue; }

            if let Some(pos) = t.find("global:") {
                mode = Some("global");
                t = t[pos + "global:".len()..].trim();
            }
            if let Some(pos) = t.find("local:") {
                mode = Some("local");
                t = t[pos + "local:".len()..].trim();
            }
            if t.is_empty() { continue; }

            // Version scripts normally list one pattern per semicolon, but allow
            // whitespace-separated patterns as a robustness improvement.
            for pat in t.split_whitespace() {
                let pat = pat.trim().trim_matches(';').trim();
                if pat.is_empty() { continue; }
                match mode {
                    Some("global") => global_patterns.push(pat.to_string()),
                    Some("local") if pat == "*" => local_star = true,
                    _ => {}
                }
            }
        }

        Some(Self { version_name, global_patterns, local_star })
    }

    pub fn matches_global(&self, name: &str) -> bool {
        self.global_patterns.iter().any(|pat| wildcard_match(pat, name))
    }
}

/// True when `kw` appears in `text` as a standalone word (not as part of a
/// longer identifier). Used to distinguish a full linker script from a
/// standalone version script.
fn contains_keyword(text: &str, kw: &str) -> bool {
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(kw) {
        let pos = from + rel;
        let before = text[..pos].chars().next_back();
        let after = text[pos + kw.len()..].chars().next();
        let boundary_before = !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        let boundary_after = !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        if boundary_before && boundary_after {
            return true;
        }
        from = pos + kw.len();
    }
    false
}

pub(crate) fn strip_version_script_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
        } else if bytes[i] == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

pub(crate) fn wildcard_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" { return true; }
    if !pattern.contains('*') { return pattern == text; }
    let mut rest = text;
    let mut first = true;
    for part in pattern.split('*') {
        if part.is_empty() { continue; }
        if first && !pattern.starts_with('*') {
            if !rest.starts_with(part) { return false; }
            rest = &rest[part.len()..];
        } else if let Some(pos) = rest.find(part) {
            rest = &rest[pos + part.len()..];
        } else {
            return false;
        }
        first = false;
    }
    pattern.ends_with('*') || rest.is_empty()
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_script(body: &str) -> String {
        let dir = std::env::temp_dir()
            .join(format!("lccc_vs_{}_{}", std::process::id(),
                          body.len() * 2654435761usize % 100000));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("v.map");
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        p.to_string_lossy().into_owned()
    }

    /// The regression that mattered: an anonymous node (no name before `{`)
    /// is the common spelling, and it used to make `parse` return None — so
    /// the script was ignored and `local: *;` exported everything.
    #[test]
    fn anonymous_version_node_is_parsed() {
        let p = write_script("{ global: exported_api; main; local: *; };\n");
        let vs = VersionScript::parse(&p).expect("anonymous node must parse");
        assert!(vs.local_star, "local: * not seen");
        assert!(vs.matches_global("exported_api"));
        assert!(vs.matches_global("main"));
        assert!(!vs.matches_global("internal_helper"));
        assert_eq!(vs.version_name, "");
    }

    #[test]
    fn named_version_node_still_works() {
        let p = write_script("LIBFOO_1.0 { global: foo_*; local: *; };\n");
        let vs = VersionScript::parse(&p).unwrap();
        assert_eq!(vs.version_name, "LIBFOO_1.0");
        assert!(vs.local_star);
        assert!(vs.matches_global("foo_bar"));
        assert!(!vs.matches_global("bar_foo"));
    }

    #[test]
    fn version_node_embedded_after_sections_is_parsed() {
        let text = r#"
            SECTIONS { .text : { *(.text) } }
            PHDRS { text PT_LOAD; }
            VERSION { LINUX_2.6 { global: clock_gettime; __vdso_clock_gettime;
                                  local: *; }; }
        "#;
        let vs = VersionScript::parse_text(text).expect("embedded VERSION node");
        assert_eq!(vs.version_name, "LINUX_2.6");
        assert!(vs.matches_global("clock_gettime"));
        assert!(vs.matches_global("__vdso_clock_gettime"));
        assert!(vs.local_star);
    }

    #[test]
    fn comments_are_stripped() {
        let p = write_script(
            "/* c comment */\n# hash comment\n{ global: keep; /* x */ local: *; };\n");
        let vs = VersionScript::parse(&p).unwrap();
        assert!(vs.matches_global("keep"));
        assert!(vs.local_star);
    }

    /// Without `local: *` nothing is restricted, so callers must not filter.
    #[test]
    fn no_local_star_means_no_restriction() {
        let p = write_script("{ global: only_this; };\n");
        let vs = VersionScript::parse(&p).unwrap();
        assert!(!vs.local_star);
    }

    #[test]
    fn wildcards_behave_like_glob() {
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("foo*", "foobar"));
        assert!(!wildcard_match("foo*", "barfoo"));
        assert!(wildcard_match("*bar", "foobar"));
        assert!(wildcard_match("foo*bar", "fooXbar"));
        assert!(!wildcard_match("foo*bar", "fooXbaz"));
        assert!(wildcard_match("exact", "exact"));
        assert!(!wildcard_match("exact", "exactly"));
    }

    #[test]
    fn missing_file_is_none_not_panic() {
        assert!(VersionScript::parse("/nonexistent/version/script").is_none());
    }
}

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

/// One `NAME { global: ...; local: ...; } [PARENT];` node.
#[derive(Debug, Clone, Default)]
pub struct VersionNode {
    pub name: String,
    pub global_patterns: Vec<String>,
    pub local_patterns: Vec<String>,
    pub local_star: bool,
    /// The inherited node, from the `NAME { ... } PARENT;` suffix.
    ///
    /// This is what makes a version *hierarchy*: `LIBV_2.0 { ... } LIBV_1.0;`
    /// tells the loader that anything satisfying LIBV_2.0 also satisfies
    /// LIBV_1.0. Dropping it produces a library that looks versioned but
    /// cannot satisfy an older dependency.
    pub parent: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VersionScript {
    pub version_name: String,
    pub global_patterns: Vec<String>,
    pub local_star: bool,
    /// Every node in declaration order.
    ///
    /// The three fields above describe the *first* node and exist because the
    /// executable path and the linker-script path only ever need "is this
    /// symbol exported?". Multi-node consumers (the shared-object verdef
    /// builder) use this vector instead.
    pub nodes: Vec<VersionNode>,
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
        // Parse EVERY node, not just the first. A version script may declare a
        // hierarchy -- `LIBV_2.0 { ... } LIBV_1.0;` -- and stopping after the
        // first node silently drops both the later versions and the inheritance
        // edges, producing a library that cannot satisfy an older dependency.
        let nodes = parse_version_nodes(text);
        let first = nodes.first().cloned().unwrap_or_default();
        Some(Self {
            version_name: first.name,
            global_patterns: first.global_patterns,
            local_star: first.local_star,
            nodes,
        })
    }

    /// True when `name` is exported by ANY node.
    ///
    /// Export is a property of the whole script, not of its first node: a
    /// symbol listed only in `LIBV_2.0` is still exported. Consulting one node
    /// silently dropped every symbol introduced by a later version.
    pub fn matches_global(&self, name: &str) -> bool {
        if self.nodes.is_empty() {
            return self.global_patterns.iter().any(|pat| wildcard_match(pat, name));
        }
        self.nodes.iter().any(|n|
            n.global_patterns.iter().any(|pat| wildcard_match(pat, name)))
    }

    /// True when `name` is explicitly matched by some node's `local:` list.
    ///
    /// `local: *` is reported through [`Self::local_star`]; this covers the
    /// narrower `local: internal_*;` form.
    pub fn matches_local(&self, name: &str) -> bool {
        self.nodes.iter().any(|n| n.local_patterns.iter()
            .any(|pat| pat != "*" && wildcard_match(pat, name)))
    }

    /// `local: *` in any node hides everything not explicitly exported.
    pub fn any_local_star(&self) -> bool {
        if self.nodes.is_empty() { return self.local_star; }
        self.nodes.iter().any(|n| n.local_star)
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

/// Split a version-script body into its nodes.
///
/// Grammar handled: `[NAME] { global: a; b; local: *; } [PARENT] ;` repeated.
/// A missing NAME is the anonymous node (visibility control only, no verdef).
/// Brace depth is tracked so a nested `extern "C++" { ... }` block cannot end
/// the node early.
fn parse_version_nodes(text: &str) -> Vec<VersionNode> {
    let bytes = text.as_bytes();
    let mut nodes = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        // Node name: everything up to the opening brace.
        let Some(rel_open) = text[i..].find('{') else { break };
        let open = i + rel_open;
        let name = text[i..open]
            .trim()
            .trim_start_matches(';')
            .trim()
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();

        // Matching close brace, tracking depth for nested blocks.
        let mut depth = 0i32;
        let mut close = None;
        for (k, ch) in text[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 { close = Some(open + k); break; }
                }
                _ => {}
            }
        }
        let Some(close) = close else { break };

        let mut node = parse_one_node_body(&text[open + 1..close]);
        node.name = name;

        // `} PARENT ;` -- the inheritance suffix.
        let after = &text[close + 1..];
        let suffix_end = after.find(';').unwrap_or(0);
        let parent = after[..suffix_end].trim();
        if !parent.is_empty() {
            node.parent = Some(parent.to_string());
        }
        nodes.push(node);

        i = close + 1 + suffix_end + usize::from(suffix_end < after.len());
        if text[i..].trim().is_empty() { break; }
    }
    nodes
}

/// Parse the `global:` / `local:` lists inside one node body.
fn parse_one_node_body(body: &str) -> VersionNode {
    let mut node = VersionNode::default();
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
                Some("global") => node.global_patterns.push(pat.to_string()),
                Some("local") => {
                    if pat == "*" { node.local_star = true; }
                    node.local_patterns.push(pat.to_string());
                }
                _ => {}
            }
        }
    }
    node
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

/// Public alias so emitters can match a pattern without owning a `VersionScript`.
pub fn wildcard_match_pattern(pattern: &str, text: &str) -> bool {
    wildcard_match(pattern, text)
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

    // ── multi-node version scripts ──────────────────────────────────────

    #[test]
    fn multiple_version_nodes_are_all_parsed_with_parents() {
        let vs = VersionScript::parse_text(
            "LIBV_1.0 { global: old_fn; local: *; };\n\
             LIBV_2.0 { global: new_fn; } LIBV_1.0;\n").unwrap();
        assert_eq!(vs.nodes.len(), 2, "both nodes must be parsed");
        assert_eq!(vs.nodes[0].name, "LIBV_1.0");
        assert_eq!(vs.nodes[1].name, "LIBV_2.0");
        assert_eq!(vs.nodes[0].parent, None);
        assert_eq!(vs.nodes[1].parent.as_deref(), Some("LIBV_1.0"),
                   "the inheritance edge is what lets a 2.0 provider satisfy a 1.0 dependency");
        // Export is a property of the whole script, not of its first node.
        assert!(vs.matches_global("old_fn"));
        assert!(vs.matches_global("new_fn"),
                "a symbol introduced by a later node is still exported");
        assert!(!vs.matches_global("internal_fn"));
        assert!(vs.any_local_star(), "local: * in any node hides the rest");
    }

    #[test]
    fn single_node_scripts_keep_their_legacy_shape() {
        // The first-node fields are what the executable and script paths read.
        let vs = VersionScript::parse_text("LIBFOO_1.0 { global: a; b; local: *; };").unwrap();
        assert_eq!(vs.version_name, "LIBFOO_1.0");
        assert_eq!(vs.global_patterns, vec!["a".to_string(), "b".to_string()]);
        assert!(vs.local_star);
        assert_eq!(vs.nodes.len(), 1);
    }

    #[test]
    fn explicit_local_patterns_are_recorded() {
        let vs = VersionScript::parse_text(
            "V { global: pub_*; local: internal_*; };").unwrap();
        assert!(vs.matches_global("pub_a"));
        assert!(vs.matches_local("internal_x"));
        assert!(!vs.matches_local("pub_a"));
        assert!(!vs.any_local_star(), "local: internal_* is not local: *");
    }

    #[test]
    fn three_node_chain_keeps_each_parent() {
        let vs = VersionScript::parse_text(
            "A_1 { global: f1; };\n\
             A_2 { global: f2; } A_1;\n\
             A_3 { global: f3; } A_2;\n").unwrap();
        let names: Vec<&str> = vs.nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["A_1", "A_2", "A_3"]);
        let parents: Vec<Option<&str>> =
            vs.nodes.iter().map(|n| n.parent.as_deref()).collect();
        assert_eq!(parents, vec![None, Some("A_1"), Some("A_2")]);
    }
}

//! Text processing utilities for the preprocessor.
//!
//! Low-level text transformations: block/line comment stripping,
//! line continuation (backslash-newline) joining, parenthesis
//! balancing checks, and directive line splitting.

use super::pipeline::Preprocessor;
use super::utils::{copy_literal_bytes_raw, skip_literal_bytes};

/// Mapping from output line numbers to original source line numbers.
///
/// After stripping block comments, output line numbers may differ from
/// source line numbers (multi-line block comments consume source lines
/// without producing output lines). This enum tracks the mapping:
///
/// - `Identity`: output line == source line (no multi-line block comments).
///   This is the common case and avoids allocating a Vec<usize>.
/// - `Mapped(Vec<usize>)`: line_map[output_line] = source_line.
/// Per-line source-position segments for exact `__LINE__` tracking.
///
/// One logical (output) line can contain bytes from several physical source
/// lines: a spliced line (backslash-newline continuation) joins parts of
/// several physical lines. C11 requires `__LINE__` to expand to the physical
/// line of the token being expanded, so a single start-line number per
/// output line is wrong for tokens that follow a splice point.
///
/// `Mapped` holds `(byte_offset, source_line)` breakpoints in ABSOLUTE
/// joined-text coordinates: the source line of the byte at `offset` is the
/// line of the last breakpoint with breakpoint offset <= `offset`.
/// `Flat(n)` is the single-line fast path.
#[derive(Debug, Clone)]
pub(super) enum LineSegments {
    /// The whole line comes from one source line.
    Flat(usize),
    /// (byte offset, 0-based source line) breakpoints.
    Mapped(std::rc::Rc<[(usize, usize)]>),
}

impl LineSegments {
    /// Source line (0-based) of the byte at `offset`.
    #[inline]
    pub(super) fn line_at(&self, offset: usize) -> usize {
        match self {
            LineSegments::Flat(n) => *n,
            LineSegments::Mapped(segs) => {
                let idx = segs.partition_point(|&(o, _)| o <= offset);
                segs[idx.saturating_sub(1)].1
            }
        }
    }
}

/// Per-output-line breakpoints produced by block-comment stripping.
///
/// `Flat(joined_line)` when the whole output line comes from one joined
/// line; `Mapped` holds `(stripped_offset, joined_offset, joined_line)`
/// breakpoints in ABSOLUTE text coordinates (byte positions in the
/// comment-stripped and line-joined texts) and `joined_line` is 0-based in
/// the already line-spliced input. The first breakpoint is the output
/// line's own start.
#[derive(Debug, Clone)]
pub(super) enum CommentSegments {
    Flat(usize),
    Mapped(std::rc::Rc<[(usize, usize, usize)]>),
}

/// Resolver that maps a byte offset to the PHYSICAL source line (0-based)
/// of that byte, for exact `__LINE__` expansion.
///
/// Two transformations shift line numbers: multi-line block comments are
/// collapsed to one space (the comment's internal newlines disappear), and
/// backslash-newline splices join several physical lines into one. The
/// comment stripper records, per output line, `(stripped_offset,
/// joined_offset, joined_line)` breakpoints in ABSOLUTE text coordinates
/// (byte positions in the comment-stripped and line-joined texts); between
/// two breakpoints the stripped and joined offsets advance in lockstep (no
/// comment bytes are removed there), so the joined position of any byte is
/// `breakpoint.joined_offset + (offset - breakpoint.stripped_offset)`. The
/// join map then resolves the joined position to a physical line.
///
/// `Flat(n)` is the fast path: no breakpoints and no splicing, the physical
/// line is simply `n` regardless of the offset.
#[derive(Debug, Clone)]
pub(super) enum LineResolver {
    /// Fast path: physical line is the given 0-based line everywhere.
    Flat(usize),
    /// Breakpoints plus the shared join map.
    Mapped(std::rc::Rc<LineResolverData>),
}

/// Shared data for a mapped line resolver.
#[derive(Debug, Clone)]
pub(super) struct LineResolverData {
    /// (stripped_offset, joined_offset, joined_line) breakpoints; always
    /// starts with (0, 0, first_joined_line).
    pub(super) points: std::rc::Rc<[(usize, usize, usize)]>,
    /// Join map (joined line -> physical segments); empty when the file has
    /// no continuations (joined lines ARE physical lines).
    pub(super) join_map: std::rc::Rc<[LineSegments]>,
}

impl LineResolver {
    /// Physical source line (0-based) of the byte at `offset`.
    #[inline]
    pub(super) fn line_at(&self, offset: usize) -> usize {
        match self {
            LineResolver::Flat(n) => *n,
            LineResolver::Mapped(data) => {
                let idx = data.points.partition_point(|&(so, _, _)| so <= offset);
                let (so, jo, jl) = data.points[idx.saturating_sub(1)];
                let joined_off = jo + (offset - so);
                data.join_map
                    .get(jl)
                    .map(|ls| ls.line_at(joined_off))
                    .unwrap_or(jl)
            }
        }
    }
}

/// Quick check for presence of comment markers (`/*` or `//`) in source bytes.
/// Scans for `/` bytes first, then checks the following byte. Since `/` is
/// relatively rare in C source (typically only in comments and division),
/// this quickly skips large stretches of non-slash bytes.
fn contains_comment_marker(bytes: &[u8]) -> bool {
    let len = bytes.len();
    if len < 2 {
        return false;
    }
    // Search for '/' bytes, then check what follows
    let mut i = 0;
    while i < len - 1 {
        // Skip non-slash bytes quickly
        if bytes[i] != b'/' {
            i += 1;
            continue;
        }
        let next = bytes[i + 1];
        if next == b'*' || next == b'/' {
            return true;
        }
        i += 1;
    }
    false
}

impl Preprocessor {
    /// Check if a line has unbalanced parentheses, indicating a multi-line
    /// macro invocation that needs to be joined with subsequent lines.
    /// Skips string/char literals and line comments.
    pub(super) fn has_unbalanced_parens(line: &str) -> bool {
        let mut depth: i32 = 0;
        let bytes = line.as_bytes();
        let len = bytes.len();
        let mut i = 0;

        while i < len {
            match bytes[i] {
                b'"' | b'\'' => {
                    i = skip_literal_bytes(bytes, i, bytes[i]);
                }
                b'(' => {
                    depth += 1;
                    i += 1;
                }
                b')' => {
                    depth -= 1;
                    i += 1;
                }
                b'/' if i + 1 < len && bytes[i + 1] == b'/' => break,
                _ => {
                    i += 1;
                }
            }
        }

        depth > 0
    }

    /// Strip C-style block comments (/* ... */) and C++ line comments (// ...).
    /// Returns the stripped source and a per-output-line [`CommentSegments`]
    /// map for correct `__LINE__` tracking.
    ///
    /// The map is `None` when no multi-line block comments shifted line
    /// numbering (the common case, avoiding any allocation) and
    /// `Some(Vec<CommentSegments>)` otherwise, with one entry per output line.
    /// Each entry is either `Flat(joined_line)` or `Mapped(breakpoints)` with
    /// `(stripped_offset, joined_offset, joined_line)` tuples, where
    /// `joined_line` is 0-based in the (already line-spliced) input text and
    /// the offsets are relative to the line start. Between two breakpoints
    /// the stripped and joined offsets advance in lockstep, so the pipeline
    /// can resolve the physical line of any byte by composing with the join
    /// map from [`Self::join_continued_lines`].
    ///
    /// Block comments are replaced with a single space (per C11 5.1.1.2
    /// phase 3), which avoids breaking preprocessor directives that have
    /// block comments between `#` and the directive keyword (e.g.,
    /// `#/*...\n...*/if 1`). Uses raw byte operations to preserve UTF-8
    /// sequences in string literals.
    pub(super) fn strip_block_comments(
        source: &str,
    ) -> (std::borrow::Cow<'_, str>, Option<Vec<CommentSegments>>) {
        let bytes = source.as_bytes();
        let len = bytes.len();

        // Fast path: if no block comment or line comment markers exist, return
        // source as-is. This avoids both the byte-by-byte scan and the
        // Vec<u8> allocation.
        if !contains_comment_marker(bytes) {
            return (std::borrow::Cow::Borrowed(source), None);
        }

        let mut result: Vec<u8> = Vec::with_capacity(source.len());
        let mut i = 0;
        // Track joined source line number (0-based) as we scan through input.
        let mut src_line: usize = 0;
        // Whether any block comment spans multiple lines (shifts line numbering).
        let mut has_multiline_block_comment = false;
        // Per output line: Flat(joined line) or Mapped(breakpoints).
        let mut line_map: Vec<CommentSegments> = Vec::new();
        // Track the joined source line at the start of the current output line.
        let mut current_output_line_src = 0usize;
        // Breakpoints of the current output line: (stripped_offset,
        // joined_offset, joined_line) in ABSOLUTE text coordinates; the
        // first entry is the output line's own start.
        let mut cur_points: Vec<(usize, usize, usize)> = vec![(0, 0, 0)];

        // Close the current output line's map entry.
        fn close_line(
            line_map: &mut Vec<CommentSegments>,
            cur_points: &mut Vec<(usize, usize, usize)>,
            current_output_line_src: usize,
        ) {
            if cur_points.len() == 1 {
                line_map.push(CommentSegments::Flat(current_output_line_src));
            } else {
                line_map.push(CommentSegments::Mapped(std::mem::take(cur_points).into()));
            }
        }

        while i < len {
            match bytes[i] {
                b'"' | b'\'' => {
                    // Copy string/char literals verbatim (don't strip comments inside)
                    let old_i = i;
                    i = copy_literal_bytes_raw(bytes, i, bytes[i], &mut result);
                    // Count newlines in the literal for source line tracking.
                    // Literal bytes are emitted verbatim, so joined offsets
                    // advance in lockstep with stripped offsets.
                    for (k, &b) in bytes[old_i..i].iter().enumerate() {
                        if b == b'\n' {
                            src_line += 1;
                            if has_multiline_block_comment {
                                close_line(&mut line_map, &mut cur_points, current_output_line_src);
                                // New output line starts right after the
                                // literal's newline: absolute stripped
                                // offset = result.len(), absolute joined
                                // offset = old_i + k + 1.
                                cur_points = vec![(result.len(), old_i + k + 1, src_line)];
                            }
                            current_output_line_src = src_line;
                        }
                    }
                }
                b'/' if i + 1 < len && bytes[i + 1] == b'*' => {
                    // Block comment - replace entire comment with a single space.
                    i += 2;
                    result.push(b' ');
                    let comment_start_line = src_line;
                    while i < len {
                        if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                            i += 2;
                            break;
                        }
                        if bytes[i] == b'\n' {
                            src_line += 1;
                        }
                        i += 1;
                    }
                    // If block comment spanned multiple lines, we need the full map.
                    if src_line > comment_start_line && !has_multiline_block_comment {
                        has_multiline_block_comment = true;
                        // Retroactively build the map for all output lines so
                        // far: they had identity mapping (output line ==
                        // joined source line).
                        let output_lines_so_far = result.iter().filter(|&&b| b == b'\n').count();
                        line_map.clear();
                        line_map.reserve(output_lines_so_far + 16);
                        for line_idx in 0..output_lines_so_far {
                            line_map.push(CommentSegments::Flat(line_idx));
                        }
                    }
                    if src_line > comment_start_line {
                        // Multi-line comment: tokens after the closing `*/`
                        // are on `src_line`, not on the line where the
                        // comment started. Record a breakpoint at the
                        // comment end (absolute stripped offset =
                        // result.len(), absolute joined offset = i) so
                        // __LINE__ resolves them correctly.
                        cur_points.push((result.len(), i, src_line));
                    }
                    // Don't emit newlines - the comment is fully replaced by one space.
                }
                b'/' if i + 1 < len && bytes[i + 1] == b'/' => {
                    // Line comment - skip to end of line.
                    i += 2;
                    while i < len && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                b'\n' => {
                    result.push(b'\n');
                    if has_multiline_block_comment {
                        close_line(&mut line_map, &mut cur_points, current_output_line_src);
                        // New output line starts at absolute stripped
                        // offset result.len() = joined offset i + 1.
                        cur_points = vec![(result.len(), i + 1, src_line + 1)];
                    }
                    src_line += 1;
                    current_output_line_src = src_line;
                    i += 1;
                }
                _ => {
                    result.push(bytes[i]);
                    i += 1;
                }
            }
        }
        // Record the last line
        if has_multiline_block_comment {
            close_line(&mut line_map, &mut cur_points, current_output_line_src);
        }

        // Input was valid UTF-8, and we only removed/replaced ASCII characters
        // (comments with spaces/newlines), so result is still valid UTF-8.
        let text = String::from_utf8(result)
            .expect("comment stripping produced non-UTF8 (input was valid UTF-8)");
        if has_multiline_block_comment {
            (std::borrow::Cow::Owned(text), Some(line_map))
        } else {
            (std::borrow::Cow::Owned(text), None)
        }
    }

    /// Join lines that end with backslash (line continuation).
    /// Also handles backslash followed by trailing whitespace before newline,
    /// matching GCC/Clang behavior (GCC warns: "backslash and newline separated by space").
    ///
    /// Returns `Cow::Borrowed` when the source has no continuation backslashes
    /// (the common case), avoiding a String allocation entirely.
    ///
    /// When any line was spliced, also returns a per-joined-line
    /// [`LineSegments`] map in PHYSICAL source-line coordinates with
    /// ABSOLUTE joined-text byte offsets, so the pipeline can resolve the
    /// physical line of tokens on spliced lines (tokens after a splice
    /// point are on a later physical line than the joined line's start).
    pub(super) fn join_continued_lines<'a>(
        &self,
        source: &'a str,
    ) -> (std::borrow::Cow<'a, str>, Option<Vec<LineSegments>>) {
        // Fast path: if the source contains no backslashes at all, there can't
        // be any line continuations. We check for '\\' rather than "\\\\n"
        // because GCC/Clang treat backslash + whitespace + newline as a
        // continuation too.
        if !source.contains('\\') {
            return (std::borrow::Cow::Borrowed(source), None);
        }

        let mut result = String::with_capacity(source.len());
        let mut continuation = false;
        let mut any_spliced = false;
        // Breakpoints of the currently open joined line: (joined_offset,
        // physical_line). A new physical part starts at the splice offset.
        let mut segs: Vec<(usize, usize)> = Vec::new();
        // Per joined line (in order): Flat(phys) or Mapped(splice points).
        let mut map: Vec<LineSegments> = Vec::new();
        // Physical line (0-based) of the physical line being processed.
        let mut phys: usize = 0;

        for line in source.lines() {
            let bs_pos = Self::find_continuation_backslash(line);
            if continuation {
                // This physical line is spliced onto the open joined line.
                if let Some(pos) = bs_pos {
                    any_spliced = true;
                    segs.push((result.len(), phys));
                    result.push_str(&line[..pos]);
                    // Still continuing.
                } else {
                    any_spliced = true;
                    segs.push((result.len(), phys));
                    map.push(LineSegments::Mapped(segs.clone().into()));
                    segs.clear();
                    result.push_str(line);
                    result.push('\n');
                    continuation = false;
                }
            } else if let Some(pos) = bs_pos {
                // Opens a new spliced joined line. The first physical part
                // starts at the joined line's ABSOLUTE start offset
                // (result.len() — all prior joined lines are already in
                // `result`); every later part records its absolute start the
                // same way, keeping the coordinate system uniform.
                any_spliced = true;
                segs.clear();
                segs.push((result.len(), phys));
                result.push_str(&line[..pos]);
                continuation = true;
            } else {
                // Standalone joined line: closes immediately.
                result.push_str(line);
                result.push('\n');
                map.push(LineSegments::Flat(phys));
            }
            phys += 1;
        }

        // A dangling continuation at EOF (GCC: "backslash-newline at end of
        // file"): close the open joined line.
        if continuation {
            any_spliced = true;
            map.push(LineSegments::Mapped(segs.clone().into()));
        }

        if any_spliced {
            (std::borrow::Cow::Owned(result), Some(map))
        } else {
            // A backslash existed but never formed a continuation (e.g.
            // inside a string literal); the text is unchanged.
            (std::borrow::Cow::Owned(result), None)
        }
    }

    /// Check if a line ends with a continuation backslash (optionally followed by whitespace).
    /// Returns the byte position of the backslash if found, None otherwise.
    pub(super) fn find_continuation_backslash(line: &str) -> Option<usize> {
        let trimmed = line.trim_end();
        if trimmed.ends_with('\\') {
            Some(trimmed.len() - 1)
        } else {
            None
        }
    }
}

/// Strip a // comment from a directive line, but not inside string literals.
/// Returns a `Cow<str>` to avoid allocation when no comment is found (the
/// common case). Only allocates a new String when a `//` comment is present
/// and needs to be stripped.
pub(super) fn strip_line_comment(line: &str) -> std::borrow::Cow<'_, str> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        match bytes[i] {
            b'"' | b'\'' => {
                i = skip_literal_bytes(bytes, i, bytes[i]);
            }
            b'/' if i + 1 < len && bytes[i + 1] == b'/' => {
                return std::borrow::Cow::Owned(line[..i].trim_end().to_string());
            }
            _ => i += 1,
        }
    }

    std::borrow::Cow::Borrowed(line)
}

/// Split a string into the first word and the rest.
/// For preprocessor directives, '(' is also a word boundary so that
/// `#if(expr)` is correctly parsed as keyword="if", rest="(expr)".
pub(super) fn split_first_word(s: &str) -> (&str, &str) {
    let s = s.trim();
    if let Some(pos) = s.find(|c: char| c.is_whitespace() || c == '(') {
        if s.as_bytes()[pos] == b'(' {
            // Don't trim the '(' - it's part of the rest
            (&s[..pos], &s[pos..])
        } else {
            (&s[..pos], s[pos..].trim())
        }
    } else {
        (s, "")
    }
}

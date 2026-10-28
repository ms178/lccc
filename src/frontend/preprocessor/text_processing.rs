//! Text processing utilities for the preprocessor.
//!
//! Low-level text transformations: block/line comment stripping,
//! line continuation (backslash-newline) joining, parenthesis
//! balancing checks, and directive line splitting.

use super::pipeline::Preprocessor;
use super::utils::{
    copy_literal_bytes_raw, hash_token_len, is_digit_separator, skip_literal_bytes,
};

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
/// Lexical state at a line splice, as far as [`join_continued_lines`] needs
/// it to decide whether the splice may be replaced by a line break.
///
/// Scanning starts afresh (`Code`) at every joined line, so a block comment
/// opened on an earlier joined line is not seen. Every misjudgement is safe:
/// taking comment text for a literal or a `//` comment only joins (the
/// conservative choice), and taking a block comment's inside for code only
/// splits inside a comment, which already spans lines.
///
/// [`join_continued_lines`]: Preprocessor::join_continued_lines
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpliceState {
    Code,
    Literal(u8),
    LineComment,
    BlockComment,
}

impl SpliceState {
    /// The state after `bytes`, starting in `self`.
    fn scan(mut self, bytes: &[u8]) -> Self {
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            match self {
                Self::Code => match (b, bytes.get(i + 1)) {
                    (b'"' | b'\'', _) if !is_digit_separator(bytes, i) => self = Self::Literal(b),
                    (b'/', Some(b'*')) => {
                        self = Self::BlockComment;
                        i += 1;
                    }
                    (b'/', Some(b'/')) => return Self::LineComment,
                    _ => {}
                },
                Self::Literal(quote) => {
                    if b == b'\\' {
                        i += 1;
                    } else if b == quote {
                        self = Self::Code;
                    }
                }
                Self::BlockComment => {
                    if b == b'*' && bytes.get(i + 1) == Some(&b'/') {
                        self = Self::Code;
                        i += 1;
                    }
                }
                Self::LineComment => return Self::LineComment,
            }
            i += 1;
        }
        self
    }
}

/// First byte of `line` that is not a space or tab.
fn first_non_blank(line: &[u8]) -> Option<u8> {
    line.iter().copied().find(|&b| b != b' ' && b != b'\t')
}

/// Whether the first token of a line is a directive introducer, recognizing
/// `%:` but not the longer `%:%:` token.
fn starts_directive(line: &[u8]) -> bool {
    let offset = line.iter().position(|&b| b != b' ' && b != b'\t');
    offset.is_some_and(|i| hash_token_len(&line[i..]) != 0)
}

/// Find a directive introducer at or after `from`, skipping horizontal space
/// and same-line block comments. A newline terminates the search.
fn next_token_is_directive(bytes: &[u8], mut from: usize) -> bool {
    loop {
        match bytes.get(from) {
            Some(b' ' | b'\t') => from += 1,
            Some(b'/') if bytes.get(from + 1) == Some(&b'*') => {
                let Some(close) = bytes[from + 2..].windows(2).position(|w| w == b"*/") else {
                    return false;
                };
                if bytes[from + 2..from + 2 + close].contains(&b'\n') {
                    return false;
                }
                from += close + 4;
            }
            Some(b'\n') | None => return false,
            _ => return hash_token_len(&bytes[from..]) != 0,
        }
    }
}

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
                b'"' | b'\'' if !is_digit_separator(bytes, i) => {
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
    /// A block comment within one line is replaced with a single space (C11
    /// 5.1.1.2 phase 3). A multi-line one on an ordinary line is replaced by
    /// the newlines it spans plus spaces up to the column after its `*/`, as
    /// `gcc -E` does: outside a directive a newline is just white space, and
    /// every token keeps its physical line AND column, which is where
    /// diagnostics and line tables point. Inside a directive (the output line
    /// so far starts with `#`) the comment stays one space, since the
    /// directive must remain one line (`#/*...\n...*/if 1`); so it does when
    /// the text after it starts with `#` on a line that already has tokens,
    /// where a split would turn a stray `#` into a directive. Only those
    /// collapsed comments shift line numbering (the pipeline re-synchronizes
    /// after the line) and need the map. Uses raw byte operations to
    /// preserve UTF-8 sequences in string literals.
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
        // Offsets in `result` and in `source` where the current output line
        // starts.
        let mut line_start = 0usize;
        let mut line_start_joined = 0usize;

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
                b'"' | b'\'' if !is_digit_separator(bytes, i) => {
                    // Copy string/char literals verbatim (don't strip comments inside)
                    let old_i = i;
                    let old_result_len = result.len();
                    i = copy_literal_bytes_raw(bytes, i, bytes[i], &mut result);
                    // Count newlines in the literal for source line tracking.
                    // Literal bytes are emitted verbatim, so joined offsets
                    // advance in lockstep with stripped offsets.
                    for (k, &b) in bytes[old_i..i].iter().enumerate() {
                        if b == b'\n' {
                            src_line += 1;
                            line_start = old_result_len + k + 1;
                            line_start_joined = old_i + k + 1;
                            if has_multiline_block_comment {
                                close_line(&mut line_map, &mut cur_points, current_output_line_src);
                                // New output line starts right after the
                                // literal's newline (copied verbatim, so
                                // at stripped old_result_len + k + 1 and
                                // joined old_i + k + 1).
                                cur_points =
                                    vec![(old_result_len + k + 1, old_i + k + 1, src_line)];
                            }
                            current_output_line_src = src_line;
                        }
                    }
                }
                b'/' if i + 1 < len && bytes[i + 1] == b'*' => {
                    let comment_start_line = src_line;
                    i += 2;
                    // Joined offset of the first byte of the physical line
                    // the comment ends on (its tail's column origin).
                    let mut end_line_start = 0usize;
                    while i < len {
                        if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                            i += 2;
                            break;
                        }
                        if bytes[i] == b'\n' {
                            src_line += 1;
                            end_line_start = i + 1;
                        }
                        i += 1;
                    }
                    let spanned = src_line - comment_start_line;
                    let line_so_far = &result[line_start..];
                    let collapse = spanned > 0
                        && (starts_directive(line_so_far)
                            || (first_non_blank(line_so_far).is_some()
                                && next_token_is_directive(bytes, i)));
                    if spanned == 0 || collapse {
                        result.push(b' ');
                    }
                    if collapse {
                        if !has_multiline_block_comment {
                            has_multiline_block_comment = true;
                            // Every earlier output line is its own joined
                            // line: ordinary multi-line comments were split.
                            let output_lines_so_far =
                                result.iter().filter(|&&b| b == b'\n').count();
                            line_map.clear();
                            line_map.reserve(output_lines_so_far + 16);
                            for line_idx in 0..output_lines_so_far {
                                line_map.push(CommentSegments::Flat(line_idx));
                            }
                            cur_points =
                                vec![(line_start, line_start_joined, current_output_line_src)];
                        }
                        // Tokens after the closing `*/` are on `src_line`,
                        // not on the line where the comment started: a
                        // breakpoint at the comment end (absolute stripped
                        // offset result.len(), joined offset i) resolves
                        // __LINE__ for them.
                        cur_points.push((result.len(), i, src_line));
                    } else if spanned > 0 {
                        // One newline per spanned line; the empty lines in
                        // between belong to the comment's inner lines.
                        for inner in comment_start_line..src_line {
                            result.push(b'\n');
                            if has_multiline_block_comment {
                                if inner == comment_start_line {
                                    close_line(
                                        &mut line_map,
                                        &mut cur_points,
                                        current_output_line_src,
                                    );
                                } else {
                                    line_map.push(CommentSegments::Flat(inner));
                                }
                            }
                        }
                        line_start = result.len();
                        line_start_joined = end_line_start;
                        // The tail keeps its column: the spaces stand in for
                        // the comment bytes on its last line, so stripped and
                        // joined offsets advance in lockstep from the line
                        // start. No padding before a line end.
                        if !matches!(bytes.get(i), None | Some(b'\n')) {
                            result.resize(result.len() + (i - end_line_start), b' ');
                        }
                        current_output_line_src = src_line;
                        if has_multiline_block_comment {
                            cur_points = vec![(line_start, end_line_start, src_line)];
                        }
                    }
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
                    line_start = result.len();
                    line_start_joined = i + 1;
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
    /// Where removing the splice provably cannot change the token sequence,
    /// the physical line break is kept instead (only the backslash goes), so
    /// the tokens after it keep their physical line and column: on an
    /// ordinary (non-directive) line, outside literals and `//` comments, with
    /// white space on one side of the splice, and with no `#` starting the
    /// next line (which would make it a directive). Otherwise the lines are
    /// joined and the pipeline re-synchronizes after the joined line.
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

        // Lexical state at the end of the open joined line's last part.
        let mut splice_state = SpliceState::Code;
        // Whether the open joined line may be split at all (not a directive).
        let mut splittable = false;
        let mut lines = source.lines().peekable();
        // Keeping the physical break (the elision above) is sound only when the
        // consumer is a C tokenizer, for which a newline is just white space.
        // The assembly consumer is an *assembler*, where a newline terminates a
        // statement: `arch/x86/boot/header.S` writes one PE Characteristics
        // expression across four physical lines
        //
        //     .word IMAGE_FILE_EXECUTABLE_IMAGE | \
        //           image_file_add_flags       | \
        //           ...
        //
        // and GCC/clang both hand `as` a SINGLE joined line for `.S` input
        // (`gcc -E -x assembler-with-cpp` — verified; `as` itself rejects the
        // continuation form, so the splice is cpp's job, not as's).  With the
        // break retained, the lccc assembler saw `.word 0x0002 |` and failed
        // with "unsupported data expression", which is what blocked the kernel
        // boot code.  Under `asm_mode` the splice is therefore always a real
        // splice.
        let may_keep_break = !self.macros.asm_mode;
        while let Some(line) = lines.next() {
            let bs_pos = Self::find_continuation_backslash(line);
            if let Some(pos) = bs_pos {
                if !continuation {
                    splice_state = SpliceState::Code;
                    splittable = first_non_blank(line.as_bytes()) != Some(b'/')
                        && !starts_directive(line.as_bytes());
                }
                splice_state = splice_state.scan(&line.as_bytes()[..pos]);
                let next = lines.peek().map_or(&b""[..], |l| l.as_bytes());
                let boundary = pos == 0
                    || matches!(line.as_bytes()[pos - 1], b' ' | b'\t')
                    || matches!(next.first(), None | Some(b' ' | b'\t'));
                if may_keep_break
                    && splittable
                    && boundary
                    && matches!(splice_state, SpliceState::Code | SpliceState::BlockComment)
                    && !starts_directive(next)
                {
                    // Keep the line break: this part ends its joined line.
                    // A block comment stays open across it (`splice_state`
                    // is reset at the next joined line's start, which is
                    // safe: see `SpliceState`). An earlier join on this
                    // joined line already set `any_spliced`; a split alone
                    // keeps the identity map.
                    if continuation {
                        segs.push((result.len(), phys));
                        map.push(LineSegments::Mapped(segs.clone().into()));
                        segs.clear();
                    } else {
                        map.push(LineSegments::Flat(phys));
                    }
                    result.push_str(&line[..pos]);
                    result.push('\n');
                    continuation = false;
                    phys += 1;
                    continue;
                }
            }
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
            b'"' | b'\'' if !is_digit_separator(bytes, i) => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_line_comments_are_split_without_a_line_map() {
        let (text, map) =
            Preprocessor::strip_block_comments("int a; /* x\n  y */ int b;\nint c;\n");
        // The tail keeps its column: 6 spaces stand for the comment's bytes
        // on its last line (`  y */`), then the source's own space.
        assert_eq!(text, "int a; \n       int b;\nint c;\n");
        assert!(map.is_none(), "output lines are the joined lines");
    }

    #[test]
    fn directive_comments_collapse_and_need_the_line_map() {
        let (text, map) =
            Preprocessor::strip_block_comments("int a;\n#if 1 /* x\n */ && 1\nint b;\n");
        assert_eq!(text, "int a;\n#if 1   && 1\nint b;\n");
        let map = map.expect("a collapsed comment shifts lines");
        assert_eq!(map.len(), 4, "one entry per output line");
        assert!(matches!(map[0], CommentSegments::Flat(0)));
        assert!(matches!(map[2], CommentSegments::Flat(3)));
    }

    #[test]
    fn line_map_activated_mid_file_starts_at_the_current_line() {
        // The first collapse happens on output line 1, whose bytes before
        // the comment are on joined line 1 (not 0, the map's initial entry).
        let (_, map) = Preprocessor::strip_block_comments("int a;\n#define X 1 /* x\n */ + 2\n");
        let map = map.expect("collapsed");
        let CommentSegments::Mapped(points) = &map[1] else {
            panic!("line 1 has a breakpoint: {:?}", map[1]);
        };
        assert_eq!(
            points[0],
            (7, 7, 1),
            "line start: stripped 7, joined 7, line 1"
        );
        assert_eq!(points[1].2, 2, "after `*/`: line 2");
    }

    #[test]
    fn splice_state_scan() {
        use SpliceState::*;
        assert_eq!(Code.scan(b"int a = 1 +"), Code);
        assert_eq!(Code.scan(b"s = \"ab"), Literal(b'"'));
        assert_eq!(Code.scan(b"s = \"a\\\"b"), Literal(b'"'), "escaped quote");
        assert_eq!(Code.scan(b"c = '\\''"), Code);
        assert_eq!(Code.scan(b"x; // note"), LineComment);
        assert_eq!(Code.scan(b"s = \"//\" "), Code, "`//` in a literal");
        assert_eq!(Code.scan(b"x /* open"), BlockComment);
        assert_eq!(Code.scan(b"x /* a */ y"), Code);
        assert_eq!(
            Literal(b'"').scan(b"cd\" + "),
            Code,
            "carried across a join"
        );
        assert_eq!(BlockComment.scan(b"still */ x"), Code);
    }
}

//! Physical source positions through preprocessing.
//!
//! The source manager maps preprocessed text back to source lines only
//! through line markers and line counting, and diagnostics and `-g` line
//! tables take their line AND column from that text. Every construct that
//! merges physical lines (block comments, line splices, multi-line macro
//! invocations, directives inside them) must therefore leave each token on
//! an output line that resolves to its physical line, at its physical
//! column where the construct allows it (GCC 16.2 is the reference: the
//! expected positions below are its diagnostics' positions).
//!
//! Marker tokens name their own position: `l7` must resolve to line 7 and
//! `l7c13` to line 7, column 13.

use super::Preprocessor;

/// `text` with every run of white space collapsed to one space.
fn squeeze(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Preprocess `src` as `pos.c`.
fn preprocess(src: &str) -> (Preprocessor, String) {
    let mut pp = Preprocessor::new();
    pp.set_filename("pos.c");
    let out = pp.preprocess(src);
    (pp, out)
}

/// A marker token's resolved (file, line, column).
#[derive(Debug)]
struct Resolved {
    file: String,
    line: usize,
    column: usize,
}

/// Resolve every marker token of the preprocessed `out` through its line
/// markers (`# N "file" [flags]` names the next line), as the source
/// manager does.
fn resolve_markers(out: &str) -> Vec<(String, Resolved)> {
    let mut found = Vec::new();
    let (mut file, mut line) = (String::new(), 1usize);
    for text in out.split('\n') {
        if let Some(rest) = text.strip_prefix("# ") {
            let mut parts = rest.splitn(2, ' ');
            if let (Some(num), Some(name)) = (parts.next(), parts.next()) {
                if let (Ok(n), Some(name)) = (num.parse::<usize>(), name.split('"').nth(1)) {
                    file = name.to_string();
                    line = n;
                    continue;
                }
            }
        }
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let ident = &text[start..i];
                if is_marker(ident) {
                    found.push((
                        ident.to_string(),
                        Resolved {
                            file: file.clone(),
                            line,
                            column: start + 1,
                        },
                    ));
                }
            } else {
                i += 1;
            }
        }
        line += 1;
    }
    found
}

/// `l<digits>` or `l<digits>c<digits>`.
fn is_marker(ident: &str) -> bool {
    let Some(rest) = ident.strip_prefix('l') else {
        return false;
    };
    let (line, column) = rest.split_once('c').unwrap_or((rest, "1"));
    !line.is_empty()
        && line.bytes().all(|b| b.is_ascii_digit())
        && !column.is_empty()
        && column.bytes().all(|b| b.is_ascii_digit())
}

/// Assert that every marker token of `src` resolves to its own position in
/// `file`, and that there are `count` of them (none lost).
#[track_caller]
fn assert_positions_in(src: &str, file: &str, count: usize) -> String {
    let (pp, out) = preprocess(src);
    assert!(pp.errors().is_empty(), "errors: {:?}", pp.errors());
    let markers = resolve_markers(&out);
    assert_eq!(markers.len(), count, "marker tokens in:\n{out}");
    for (ident, at) in &markers {
        let rest = &ident[1..];
        let (line, column) = match rest.split_once('c') {
            Some((l, c)) => (
                l.parse::<usize>().unwrap(),
                Some(c.parse::<usize>().unwrap()),
            ),
            None => (rest.parse::<usize>().unwrap(), None),
        };
        assert_eq!(
            (at.file.as_str(), at.line),
            (file, line),
            "`{ident}` resolves to {at:?} in:\n{out}"
        );
        if let Some(column) = column {
            assert_eq!(at.column, column, "`{ident}` column in:\n{out}");
        }
    }
    out
}

#[track_caller]
fn assert_positions(src: &str, count: usize) -> String {
    assert_positions_in(src, "pos.c", count)
}

#[test]
fn multi_line_comment_on_an_ordinary_line_keeps_the_tail_line_and_column() {
    // GCC: the tail token keeps 2:13; later lines are unshifted.
    assert_positions("int l1 = 0; /* a\n   b */ int l2c13;\nint l3;\n", 3);
}

#[test]
fn comment_spanning_many_lines_keeps_every_later_line() {
    assert_positions(
        "/* a\n\n\n\n */ int l5c9;\nint l6 = /* x\n y\n z */ l8c7;\nint l9;\n",
        4,
    );
}

#[test]
fn comment_in_a_directive_collapses_and_the_next_line_resynchronizes() {
    let out = assert_positions("#define A 1 /* x\n y */ + 2\nint l3 = A;\nint l4;\n", 2);
    assert!(squeeze(&out).contains("int l3 = 1 + 2;"), "{out}");
}

#[test]
fn hash_after_a_comment_on_a_line_with_tokens_stays_a_stray_hash() {
    // C: `#` is not the first token of its line, so there is no directive;
    // splitting the comment would have created one.
    let (pp, out) = preprocess("int l1; /* x\n */ #define Q 1\nint l3 = Q;\n");
    assert!(!pp.macros.is_defined("Q"), "{out}");
    assert!(out.contains("#define Q 1"), "{out}");
    assert!(out.contains("int l3 = Q;"), "{out}");
}

#[test]
fn hash_after_a_comment_opening_a_line_is_a_directive() {
    // `/* x */ #define` at the start of a line is a directive (the comment is
    // white space before `#`), across lines too.
    let (pp, out) = preprocess("/* x\n */ #define Q 1\nint l3 = Q;\n");
    assert!(pp.macros.is_defined("Q"), "{out}");
    assert!(out.contains("int l3 = 1;"), "{out}");
}

#[test]
fn splice_at_white_space_on_an_ordinary_line_keeps_the_tail_line_and_column() {
    // GCC: 2:2 and 4:3; the splice is replaced by the line break.
    assert_positions(
        "int l1 = 1 + \\\n l2c2;\nint f(void){ int y = 1 +\\\n  l4c3; return 0; }\nint l5;\n",
        4,
    );
}

#[test]
fn splice_in_a_directive_is_joined_and_the_next_line_resynchronizes() {
    let out = assert_positions("#define D 1 + \\\n 2\nint l3 = D;\nint l4;\n", 2);
    assert!(squeeze(&out).contains("int l3 = 1 + 2;"), "{out}");
}

#[test]
fn splices_that_change_tokens_are_joined() {
    // Inside a literal, inside a `//` comment (the splice extends it), and
    // mid-token: removing the splice is the only correct reading.
    let (_, out) = preprocess(
        "const char *s = \"ab\\\ncd\";\n// gone \\\nint hidden;\nint fo\\\no;\nint i = +\\\n+j;\nint l9;\n",
    );
    assert!(out.contains("\"abcd\""), "{out}");
    assert!(!out.contains("hidden"), "{out}");
    assert!(out.contains("int foo;"), "{out}");
    assert!(out.contains("++j"), "{out}");
    assert_positions(
        "const char *s = \"ab\\\ncd\";\n// gone \\\nint hidden;\nint fo\\\no;\nint i = +\\\n+j;\nint l9;\n",
        1,
    );
}

#[test]
fn splice_before_a_line_starting_with_hash_is_joined() {
    // Splitting would turn the `#` into a directive.
    let (pp, out) = preprocess("int a = 1 \\\n #define Q 1\n");
    assert!(!pp.macros.is_defined("Q"), "{out}");
}

#[test]
fn multi_line_plain_call_does_not_shift_later_lines() {
    // Accumulated text kept its line breaks AND was padded, doubling them.
    assert_positions("int x = f(1,\n  2) + l2c8;\nint l3;\nint l4;\n", 3);
}

#[test]
fn multi_line_macro_invocation_tail_keeps_line_and_column() {
    assert_positions(
        "#define ADD(a, b) ((a) + (b))\nint q = ADD(1,\n 2) + l3c7;\nint l4;\n",
        2,
    );
}

#[test]
fn line_breaks_inside_arguments_are_counted_once() {
    // A break inside an argument survives argument expansion; the outermost
    // invocation must not re-emit it as well.
    assert_positions(
        "#define ID(x) x\n#define TWO(a, b) a b\nint n = ID(ID(1) +\n ID(2)) + l4c11;\nint t = TWO(1\n , +\n 0) + l7c7;\nint l8;\n",
        3,
    );
}

#[test]
fn directive_lines_inside_an_invocation_are_counted() {
    assert_positions(
        "#define F(x) x\nint v = F(1\n#if 1\n + 2\n#else\n + 3\n#endif\n ) + l8c6;\nint l9;\n",
        2,
    );
}

#[test]
fn line_macro_in_multi_line_invocations_matches_gcc() {
    // GCC 16.2: an argument's __LINE__ is its own line; a body's is the line
    // of the invocation's name (Clang agrees except for TWO(L\n())).
    let (_, out) = preprocess(
        "#define ADD(a, b) a + b\n#define L() __LINE__\n#define TWO(a, b) a b\n\
         int x = ADD(__LINE__,\n __LINE__);\n\
         int y = L\n ();\n\
         int z = ADD(L(),\n L());\n\
         int t = TWO(L\n (), +\n 0);\n",
    );
    for expected in [
        "int x = 4 + 5",
        "int y = 6",
        "int z = 8 + 9",
        "int t = 10 +",
    ] {
        assert!(out.contains(expected), "missing `{expected}` in:\n{out}");
    }
}

#[test]
fn line_directive_moves_later_tokens() {
    assert_positions_in(
        "int a;\n#line 100 \"gram.y\"\nint l100c5;\nint l101;\n",
        "gram.y",
        2,
    );
    let (_, out) = preprocess("int a;\n#line 100\nint b = __LINE__;\n");
    assert!(out.contains("int b = 100;"), "{out}");
}

#[test]
fn line_directive_inside_an_invocation_moves_its_tail() {
    // GCC: the tail after `)` is on presumed line 50 (physical 4).
    assert_positions(
        "#define ADD(a, b) ((a) + (b))\nint f(void){ return ADD(1,\n#line 50\n 2) + l50c7; }\nint l51;\n",
        2,
    );
}

/// Preprocess `src` in assembly mode (`.S` / `-x assembler-with-cpp`).
fn preprocess_asm(src: &str) -> (Preprocessor, String) {
    let mut pp = Preprocessor::new();
    pp.set_filename("unit.S");
    pp.set_asm_mode(true);
    let out = pp.preprocess(src);
    (pp, out)
}

#[test]
fn assembly_splices_are_real_splices() {
    // arch/x86/boot/header.S spells ONE PE Characteristics expression over four
    // physical lines.  GCC and clang both give `as` a single joined line for
    // `.S` input (`gcc -E -x assembler-with-cpp`), and `as` itself rejects the
    // continuation form — so the splice must be cpp's.  C mode deliberately
    // keeps the break (the elision in join_continued_lines); assembly mode must
    // not, or the assembler sees a truncated `.word 0x0002 |` expression.
    let (_, out) = preprocess_asm(
        "\t.word\t0x0002\t| \\\n\
         \t\t0x0400\t| \\\n\
         \t\t0x0004\n\
         \t.long\t1\n",
    );
    let body: Vec<&str> = out
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .collect();
    assert_eq!(
        body,
        vec!["\t.word\t0x0002\t| \t\t0x0400\t| \t\t0x0004", "\t.long\t1"],
        "assembly continuation must collapse to one line"
    );
    assert!(
        !out.contains("0x0002\t|\n"),
        "a splice was split into two assembly lines:\n{out}"
    );
}

#[test]
fn c_splices_still_keep_their_break() {
    // The elision under test is the C-mode one: it keeps the physical break
    // (so later tokens keep their physical line/column) as long as the break
    // cannot change the token sequence.
    let (_, out) = preprocess("int a = 1 + \\\n 2;\n");
    assert!(
        out.contains("1 + \n 2;"),
        "C-mode splice elision changed shape:\n{out}"
    );
}

// ---- comment-bridged directives (fs/nfs/callback_xdr.c regression) ----------
//
// C11 5.1.1.2 orders the phases: splicing (phase 2) before comment
// replacement (phase 3) before directives (phase 4). A comment that opens on
// a directive line and spans that line's *unspliced* newline — a newline the
// comment itself contains, so phase 2 never touched it — merges the next
// physical line into the same directive at phase 3. A backslash after the
// comment closes must still splice, and no keep-break elision may insert a
// directive-terminating newline anywhere inside that extent. The shape is
// the kernel's fs/nfs/callback_xdr.c CB_OP_GETATTR_RES_MAXSZ macro; the
// miscompile leaked `(2 + 2 + 3 + 3 + 3 + 3 + 3) * 4)` as stray tokens
// ("expected declaration before '('") and truncated the #define.

#[test]
fn comment_bridged_directive_with_post_comment_splice_stays_whole() {
    let src = concat!(
        "#define HDR (2 * 4)\n",
        "#define RES (HDR + \\\n",
        "  /* change, size, atime,\n",
        "   * mtime, deleg_atime */\\\n",
        "  (2 + 2 + 3 + 3) * 4)\n",
        "int arr[RES];\n",
        "int after;\n",
    );
    let (pp, out) = preprocess(src);
    assert!(pp.errors().is_empty(), "errors: {:?}", pp.errors());
    assert!(pp.macros.is_defined("RES"), "{out}");
    assert!(
        squeeze(&out).contains("int arr[((2 * 4) + (2 + 2 + 3 + 3) * 4)];"),
        "macro body truncated or arithmetic leaked:\n{out}"
    );
}

#[test]
fn comment_bridged_directive_apostrophe_and_slash_text_stays_whole() {
    // Comment text is not code on the bridged line: the apostrophe in
    // `it's` and the `//` sequence must not flip the lexical state the
    // joiner scans with (they would if the state were rescanned from
    // `Code` instead of inherited as `BlockComment`).
    let src = concat!(
        "#define RES (1 + \\\n",
        "  /* it's not // code\n",
        "   * still comment */ 2 + \\\n",
        "  3)\n",
        "int arr[RES];\n",
    );
    let (pp, out) = preprocess(src);
    assert!(pp.errors().is_empty(), "errors: {:?}", pp.errors());
    assert!(pp.macros.is_defined("RES"), "{out}");
    assert!(
        squeeze(&out).contains("int arr[(1 + 2 + 3)];"),
        "quoted comment text corrupted the bridged directive:\n{out}"
    );
}

#[test]
fn comment_bridge_poisons_directives_only_not_ordinary_lines() {
    // Ordinary-line bridge (no directive): tokens still merge and the
    // markers keep their physical lines — the fix must not poison
    // ordinary code with the directive-context carry.
    let src = concat!("int l1 = 1 + /* c\n", "  c */ 2;\n", "l3c1;\n", "l4c1;\n",);
    let _ = assert_positions(src, 3);
    let (_, out) = preprocess(src);
    assert!(
        squeeze(&out).contains("int l1 = 1 + 2;"),
        "ordinary comment bridge broke tokens:\n{out}"
    );
}

#[test]
fn comment_bridge_directive_state_ends_at_comment_close() {
    // The bridge closes on the second line; a LATER ordinary splice must
    // still keep its break — directive context must not leak past the
    // comment that carried it, and the carry must not stick either.
    let src = concat!(
        "#define D (1 + /*\n",
        "  c */ 2)\n",
        "int a = 1 + \\\n",
        " 2;\n",
    );
    let (pp, out) = preprocess(src);
    assert!(pp.errors().is_empty(), "errors: {:?}", pp.errors());
    assert!(pp.macros.is_defined("D"), "{out}");
    assert!(
        out.contains("1 + \n 2;"),
        "ordinary splice after a closed bridge lost its break:\n{out}"
    );
    assert!(
        squeeze(&out).contains("int a = 1 + 2;"),
        "ordinary splice after a closed bridge broke tokens:\n{out}"
    );
}

#[test]
fn comment_bridged_directive_resyncs_positions_afterwards() {
    // The merged directive collapses to one output line; the markers on
    // the following physical lines must still resolve to their own lines
    // and column 1 (no drift from the merge).
    let src = concat!(
        "#define RES (1 + \\\n",
        "/* a\n",
        "b */ 2)\n",
        "l4c1 + RES;\n",
        "l5c1;\n",
    );
    assert_positions(src, 2);
}

// ---- comment bridges that start on an ORDINARY line ------------------------
// The comment's *opening* line says nothing about the phase-4 context of the
// line it bridges into; only the closing line's remaining tokens do. Reading
// the opening line instead is what truncated `#define … 1 + \` here.

#[test]
fn comment_bridge_from_an_ordinary_line_into_a_directive_stays_whole() {
    // GCC: the comment's newline is deleted, the `#define` it precedes is a
    // directive, and its splice joins `2`, so `A` is `1 + 2`.
    let src = concat!(
        "/* header\n",
        "   comment */ #define A 1 + \\\n",
        "  2\n",
        "int arr[A];\n",
    );
    let (pp, out) = preprocess(src);
    assert!(pp.errors().is_empty(), "errors: {:?}", pp.errors());
    assert!(pp.macros.is_defined("A"), "{out}");
    assert!(
        squeeze(&out).contains("int arr[1 + 2];"),
        "directive bridged from an ordinary line was truncated:\n{out}"
    );
}

#[test]
fn comment_bridge_into_a_directive_without_space_after_the_close() {
    // `*/#define` — the introducer is the first byte after the terminator.
    let src = concat!(
        "/* header\n",
        "   comment */#define B 2 * \\\n",
        "  3\n",
        "int arr[B];\n",
    );
    let (pp, out) = preprocess(src);
    assert!(pp.errors().is_empty(), "errors: {:?}", pp.errors());
    assert!(pp.macros.is_defined("B"), "{out}");
    assert!(
        squeeze(&out).contains("int arr[2 * 3];"),
        "`*/#define` was not treated as a directive:\n{out}"
    );
}

#[test]
fn a_bridged_comment_swallows_following_code_into_the_macro_body() {
    // The carry must reproduce phase 3 exactly, including its destructive
    // half: GCC deletes the comment's newline, so `int swallowed …;` is part
    // of the directive and never reaches the output. It is not code the fix
    // may "rescue", and the macro body keeps its spliced continuation.
    let src = concat!(
        "#define D 1 /* c\n",
        "*/ int swallowed = 1 \\\n",
        "  + 2;\n",
        "int after;\n",
    );
    let (pp, out) = preprocess(src);
    assert!(pp.errors().is_empty(), "errors: {:?}", pp.errors());
    assert!(pp.macros.is_defined("D"), "{out}");
    assert!(!out.contains("swallowed"), "code must stay macro body:\n{out}");
    assert!(squeeze(&out).contains("int after;"), "{out}");
}

#[test]
fn a_bridged_ordinary_line_keeps_its_break() {
    // The carry must not turn every comment-bridged line into a join: a line
    // that starts inside a comment but is not directive context is still
    // splittable, so the keep-break elision — which preserves __LINE__
    // granularity for consumers that can take it — survives unchanged.
    let src = concat!("int a = 1 + /* c\n", "*/ 2 + \\\n", " 3;\n");
    let (pp, out) = preprocess(src);
    assert!(pp.errors().is_empty(), "errors: {:?}", pp.errors());
    assert!(
        out.contains("2 + \n 3;"),
        "an ordinary bridged line lost its kept break:\n{out:?}"
    );
    assert!(squeeze(&out).contains("int a = 1 + 2 + 3;"), "{out}");
}

#[test]
fn comment_bridge_from_an_ordinary_line_resyncs_positions() {
    // The merged directive collapses two physical lines into one output line;
    // the marker after it still resolves to its own line and column.
    let src = concat!(
        "/* header\n",
        "   comment */ #define A 1 \\\n",
        "  + 2\n",
        "l4c1 + A;\n",
        "l5c1;\n",
    );
    assert_positions(src, 2);
}

//! Whitespace invariance of the peephole pipeline.
//!
//! Trailing whitespace is semantically neutral in every assembly syntax GAS
//! accepts, so a pipeline that behaves differently on a padded line is wrong:
//! there is no input for which the padding is the point.  That makes it a
//! testable invariant over REAL assembly rather than a list of sites to
//! inspect: run the whole pipeline on a text and on the same text with every
//! line padded, and require the two outputs to agree once the padding is
//! stripped.
//!
//! It is not hypothetical.  `LineInfo::trimmed` hands a pass the raw line slice
//! from the cached leading-trim offset onward — trailing blanks included — and
//! a pass that derives an operand by cutting `dest_token.len()` bytes off that
//! slice silently shifts the boundary when the line carries trailing blanks.
//! The address-copy kill test in `local_patterns.rs` had exactly that bug (a
//! genuine full-width kill was reported as a use, so the fold was lost), and
//! the class was fixed at one site while `len() - ` arithmetic remained at
//! dozens of others.  This module is the check that finds the rest, and the
//! reason a new one cannot land silently.
//!
//! The corpus is the repository's own committed assembly plus, when
//! `LCCC_ASM_CORPUS` names a directory, freshly generated compiler output (see
//! `tests/regression/check_peephole_whitespace.sh`).  Both are asserted
//! non-empty: a whitespace test over no assembly passes vacuously, which is the
//! failure mode this crate's other gates explicitly guard against.

use super::peephole_optimize;
use crate::backend::peephole_common::LineStore;

/// Trailing padding variants.  Each is neutral to GAS and each reaches a
/// different assumption: one blank (the off-by-one), four (the width at which a
/// `dest_token.len()` window reaches back into the destination itself), a tab (a
/// different byte, same class), eight blanks (a long tail), a bare CR and a
/// blank-plus-CR (a file with CRLF endings, which survives any split on `\n`),
/// and a mixed tail.
/// How many failures get the (expensive) minimized reproduction; every failure
/// still appears in the summary table.
const DETAIL_LIMIT: usize = 6;

const PADDINGS: [&str; 7] = [" ", "    ", "\t", "        ", "\r", " \r", "  \t "];

/// Split on `\n` WITHOUT dropping a trailing `\r`.  `str::lines()` treats
/// `\r\n` as a single terminator and strips the carriage return, which would
/// erase the very byte the CRLF variant tests: reports would show an unpadded
/// line and `unpad_lines` would silently become a no-op.
fn split_lines(text: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = text.split('\n').collect();
    if parts.last().is_some_and(|last| last.is_empty()) {
        parts.pop();
    }
    parts
}

/// Strip trailing whitespace from every line so two runs are comparable.
fn normalize(asm: &str) -> Vec<String> {
    split_lines(asm)
        .into_iter()
        .map(|line| line.trim_end().to_string())
        .collect()
}

/// Pad every line of `asm` with `padding`.
fn pad(asm: &str, padding: &str) -> String {
    let mut out = String::with_capacity(asm.len() + split_lines(asm).len() * padding.len() + 1);
    for line in split_lines(asm) {
        out.push_str(line);
        out.push_str(padding);
        out.push('\n');
    }
    out
}

/// Render lines back into assembly text.  Both sides of every comparison go
/// through this one function, which is what makes the comparison measure the
/// pipeline instead of the splitter: `LineStore::new` splits on `\n` and
/// deliberately does not let a trailing newline create an extra empty line, so
/// a text built by `join("\n")` alone parses as one line fewer whenever its
/// last line is empty.  A whitespace-only line whose padding is stripped is
/// exactly that case, and an earlier revision of this harness "minimized" a
/// real difference in `work/sha256_lccc.s` down to a two-blank-line artifact
/// because the two sides had different line counts.
fn render(lines: &[&str]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// The lines of `text` with one trailing `padding` removed from each.
fn unpad_lines<'a>(lines: &[&'a str], padding: &str) -> Vec<&'a str> {
    lines
        .iter()
        .map(|line| line.strip_suffix(padding).unwrap_or(line))
        .collect()
}

/// Run the pipeline, turning a panic into a reportable outcome: one crashing
/// input must not hide the rest of the corpus, and "the padded run panicked
/// while the plain run did not" is precisely the finding worth printing.
fn try_peephole(text: &str) -> Result<String, String> {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let owned = text.to_string();
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| peephole_optimize(owned)));
    std::panic::set_hook(prev);
    match out {
        Ok(text) => Ok(text),
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_else(|| "<non-string panic>".to_string());
            Err(format!("PANIC: {msg}"))
        }
    }
}

/// Does this padded text disagree with its own unpadded form?  Self-contained
/// on purpose: a predicate that compares against some larger baseline never
/// shrinks, because every smaller window differs from the bigger output.
fn is_sensitive(padded_text: &str, padding: &str) -> bool {
    let padded_lines = split_lines(padded_text);
    let plain_lines = unpad_lines(&padded_lines, padding);
    match (
        try_peephole(&render(&plain_lines)),
        try_peephole(&render(&padded_lines)),
    ) {
        (Ok(plain), Ok(padded)) => normalize(&plain) != normalize(&padded),
        (Err(a), Err(b)) => a != b,
        _ => true,
    }
}

/// The first difference between two normalized outputs, with context.
fn first_difference(base: &[String], other: &[String]) -> Option<String> {
    let n = base.len().max(other.len());
    for i in 0..n {
        let b = base.get(i).map(String::as_str).unwrap_or("<absent>");
        let o = other.get(i).map(String::as_str).unwrap_or("<absent>");
        if b == o {
            continue;
        }
        let mut report = format!("first difference at line {} of {}:\n", i + 1, n);
        for j in i.saturating_sub(3)..(i + 4).min(n) {
            let mark = if j == i { ">>" } else { "  " };
            report.push_str(&format!(
                "{mark} {:4}  unpadded: {:?}\n{mark}       padded:   {:?}\n",
                j + 1,
                base.get(j).map(String::as_str).unwrap_or("<absent>"),
                other.get(j).map(String::as_str).unwrap_or("<absent>"),
            ));
        }
        return Some(report);
    }
    None
}

/// Smallest contiguous line window of `padded_text` that still reproduces the
/// sensitivity, with the padding then removed from every line that does not
/// need it — so a failure names the blanks that matter instead of a whole file.
fn minimize(padded_text: &str, padding: &str, budget: &mut usize) -> String {
    let lines = split_lines(padded_text);
    let mut sensitive = |text: &str| -> bool {
        if *budget == 0 {
            return false; // out of probes: stop shrinking, keep what we have
        }
        *budget -= 1;
        is_sensitive(text, padding)
    };
    if lines.is_empty() || !sensitive(&render(&lines)) {
        return render(&lines);
    }
    let mut start = 0usize;
    let mut end = lines.len();
    loop {
        if start + 1 >= end {
            break;
        }
        if sensitive(&render(&lines[start + 1..end])) {
            start += 1;
            continue;
        }
        if sensitive(&render(&lines[start..end - 1])) {
            end -= 1;
            continue;
        }
        break;
    }
    // Then drop the padding from every line that does not need it, so the
    // report shows exactly which lines carry the significant blanks.
    let mut current: Vec<&str> = lines[start..end].to_vec();
    for idx in 0..current.len() {
        let Some(trimmed) = current[idx].strip_suffix(padding) else {
            continue;
        };
        let mut candidate = current.clone();
        candidate[idx] = trimmed;
        if sensitive(&render(&candidate)) {
            current = candidate;
        }
    }
    render(&current)
}

/// Render a minimized window so the significant blanks are visible: space as
/// `_`, tab as `>`, carriage return as `~`.
fn reveal(minimal: &str) -> String {
    minimal
        .chars()
        .map(|c| match c {
            ' ' => '_',
            '\t' => '>',
            '\r' => '~',
            other => other,
        })
        .collect()
}

/// Assembly shapes the committed corpus may not contain, written so each one
/// exercises a different operand-slicing path: a SIB source and destination, an
/// rbp-relative slot, a self-zeroing kill, a sub-width write, a high-byte alias,
/// a setcc, a cmov, a push/pop pair, an inline-asm region, a label and a
/// directive, and a jump whose target is a local label.
const FIXTURES: [&str; 6] = [
    // The address-copy fold and its kill test, with the shapes that decide it.
    concat!(
        ".text\n",
        "f:\n",
        ".cfi_startproc\n",
        "    pushq %rbp\n",
        "    movq %rsp, %rbp\n",
        "    movq %rdi, %rcx\n",
        "    movq (%rcx), %rax\n",
        "    movq %rbx, %rcx\n",
        "    movq %rcx, -16(%rbp)\n",
        "    movzbl %ch, %ecx\n",
        "    movw %ax, %cx\n",
        "    sete %cl\n",
        "    xorl %ecx, %ecx\n",
        "    popq %rcx\n",
        "    cmovne %rax, %rcx\n",
        "    movq (%rbp,%rcx,4), %rax\n",
        "    leaq 8(%rcx), %rdx\n",
        "    popq %rbp\n",
        "    ret\n",
        ".cfi_endproc\n",
        ".size f, .-f\n",
    ),
    // Control flow, local labels and a computed jump target.
    concat!(
        ".text\n",
        "g:\n",
        ".cfi_startproc\n",
        "    xorl %eax, %eax\n",
        ".LBB0_1:\n",
        "    addq $1, %rax\n",
        "    cmpq $10, %rax\n",
        "    jl .LBB0_1\n",
        "    testl %eax, %eax\n",
        "    je .LBB0_3\n",
        "    jmp *.LJTI0_0(,%rax,8)\n",
        ".LBB0_3:\n",
        "    ret\n",
        ".cfi_endproc\n",
    ),
    // An inline-asm region: everything between the markers is opaque and
    // pinned, so padding inside it must not change what the passes do around it.
    concat!(
        ".text\n",
        "h:\n",
        ".cfi_startproc\n",
        "    movq %rdi, %rax\n",
        "#APP\n",
        "    movzbl %ch, %ecx\n",
        "    xorl %ecx, %ecx\n",
        "#NO_APP\n",
        "    addq %rax, %rdi\n",
        "    ret\n",
        ".cfi_endproc\n",
    ),
    // Directives, sections and symbol arithmetic: the assembler's own operand
    // shapes, where a bare numeric literal must not be mistaken for a label.
    concat!(
        ".section .rodata\n",
        ".Lstr:\n",
        "    .asciz \"padded\"\n",
        ".section .data\n",
        "    .quad 0xffffffff80000000\n",
        "    .quad init_top_pgt - __START_KERNEL_map\n",
        "    .long 0x1f\n",
        "    .byte 0644\n",
        ".text\n",
        "    .p2align 4\n",
        "k:\n",
        "    leaq .Lstr(%rip), %rdi\n",
        "    call puts@PLT\n",
        "    ret\n",
    ),
    // Vector and wide forms: a different destination spelling per line.
    concat!(
        ".text\n",
        "v:\n",
        ".cfi_startproc\n",
        "    vmovdqa (%rdi), %xmm0\n",
        "    vpaddd %xmm1, %xmm0, %xmm2\n",
        "    vmovdqu %ymm3, (%rsi)\n",
        "    vextracti128 $1, %ymm3, %xmm4\n",
        "    vzeroupper\n",
        "    ret\n",
        ".cfi_endproc\n",
    ),
    // A callee-saved shuffle and stack-slot rewrites: the shapes the
    // offset-tracking passes slice operands for, plus the immediate-load /
    // ALU pair whose destination is looked up in a 16-entry register table.
    concat!(
        ".text\n",
        "w:\n",
        ".cfi_startproc\n",
        "    pushq %rbx\n",
        "    pushq %r12\n",
        "    pushq %r13\n",
        "    movq %rdi, %rbx\n",
        "    movq %rsi, %r12\n",
        "    movq %rbx, -8(%rbp)\n",
        "    movq -8(%rbp), %r13\n",
        "    movq %r13, -16(%rbp)\n",
        "    addq %r12, %r13\n",
        "    movq %r13, %rax\n",
        "    movl $1, %eax\n",
        "    addl $2, %eax\n",
        "    movq $3, %xmm0\n",
        "    popq %r13\n",
        "    popq %r12\n",
        "    popq %rbx\n",
        "    ret\n",
        ".cfi_endproc\n",
    ),
];

/// Every assembly text in the corpus: the fixtures, the repository's committed
/// `.s` files, and — when `LCCC_ASM_CORPUS` names a directory — the freshly
/// generated compiler output the regression script supplies.
fn corpus() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = FIXTURES
        .iter()
        .enumerate()
        .map(|(i, text)| (format!("fixture[{i}]"), (*text).to_string()))
        .collect();

    let manifest = env!("CARGO_MANIFEST_DIR");
    out.extend(collect_asm(
        std::path::Path::new(manifest),
        &["tests", "work"],
    ));

    if let Some(dir) = std::env::var_os("LCCC_ASM_CORPUS") {
        let generated = collect_asm(std::path::Path::new(&dir), &[]);
        assert!(
            !generated.is_empty(),
            "LCCC_ASM_CORPUS={} produced no assembly; a whitespace test over an \
             empty corpus passes vacuously",
            dir.to_string_lossy()
        );
        out.extend(generated);
    }
    out
}

/// Walk `root` (optionally through `subdirs`) and read every `.s` file, sorted
/// so a failure names the same file on every run.  Depth is capped so a
/// self-referential symlink in a scratch directory cannot hang the suite.
fn collect_asm(root: &std::path::Path, subdirs: &[&str]) -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, depth: u32, paths: &mut Vec<std::path::PathBuf>) {
        if depth > 12 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, depth + 1, paths);
            } else if path.extension().is_some_and(|ext| ext == "s") {
                paths.push(path);
            }
        }
    }

    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    if subdirs.is_empty() {
        walk(root, 0, &mut paths);
    } else {
        for sub in subdirs {
            walk(&root.join(sub), 0, &mut paths);
        }
    }
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            // Nothing to learn from an empty text, and a binary that happens to
            // end in .s is not assembly.
            if text.trim().is_empty() || text.contains('\0') {
                return None;
            }
            let label = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            Some((label, text))
        })
        .collect()
}

#[test]
fn the_corpus_is_not_vacuous() {
    let corpus = corpus();
    let lines: usize = corpus.iter().map(|(_, text)| text.lines().count()).sum();
    assert!(
        corpus.len() >= FIXTURES.len() + 20,
        "corpus holds only {} texts; the repository's committed assembly was not found",
        corpus.len()
    );
    assert!(
        lines > 2_000,
        "corpus holds only {lines} lines of assembly; too little to be evidence"
    );
}

#[test]
fn peephole_output_is_invariant_to_trailing_whitespace() {
    let corpus = corpus();
    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut summary: Vec<String> = Vec::new();

    for (label, text) in corpus.iter() {
        for padding in PADDINGS {
            let padded_text = pad(text, padding);
            // The padding must actually reach the text, or the variant proves
            // nothing (a corpus of empty files would silently skip it).
            assert!(
                padded_text.len() > text.len(),
                "padding {padding:?} did not enlarge {label}"
            );
            checked += 1;
            if !is_sensitive(&padded_text, padding) {
                continue;
            }
            // Whole-text probe first (cheap and always accurate), then the
            // expensive shrink, paid for only on the failures we print.
            let plain_full = try_peephole(text);
            let padded_full = try_peephole(&padded_text);
            summary.push(format!(
                "  {label:<40} padding={padding:<10?} {:<5} | {}",
                if padded_full.is_err() {
                    "PANIC"
                } else {
                    "diff"
                },
                match (&plain_full, &padded_full) {
                    (Ok(a), Ok(b)) => first_difference(&normalize(a), &normalize(b))
                        .and_then(|d| d.lines().next().map(str::to_string))
                        .unwrap_or_default(),
                    (Ok(_), Err(e)) => e.clone(),
                    (Err(e), Ok(_)) => format!("plain panicked: {e}"),
                    (Err(a), Err(b)) => format!("both panicked differently: {a} vs {b}"),
                }
            ));
            if failures.len() >= DETAIL_LIMIT {
                continue;
            }
            let mut budget = 600usize;
            let minimal = minimize(&padded_text, padding, &mut budget);
            let minimal_lines = split_lines(&minimal);
            let plain = try_peephole(&render(&unpad_lines(&minimal_lines, padding)));
            let padded = try_peephole(&render(&minimal_lines));
            let detail = match (&plain, &padded) {
                (Ok(a), Ok(b)) => first_difference(&normalize(a), &normalize(b))
                    .unwrap_or_else(|| "outputs differ only in line count".to_string()),
                _ => String::new(),
            };
            failures.push(format!(
                "{label} with trailing {padding:?}\n\
                 smallest reproduction (blanks revealed):\n{}\n\
                 unpadded: {plain:?}\n\
                 padded:   {padded:?}\n{detail}",
                split_lines(&minimal)
                    .into_iter()
                    .map(|line| format!("                   {line}    [{}]", reveal(line)))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{n} of {checked} padded runs changed the output.\n\n\
         every failure:\n{table}\n\nfirst {shown} in detail:\n\n{reports}",
        n = failures.len(),
        table = summary.join("\n"),
        shown = failures.len().min(DETAIL_LIMIT),
        reports = failures
            .iter()
            .take(DETAIL_LIMIT)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    assert!(
        checked >= corpus.len() * PADDINGS.len(),
        "only {checked} padded runs; every text must run under every padding"
    );
}

#[test]
fn the_padding_is_stopped_at_the_store_boundary() {
    // Positive control on the harness, in both directions.  An invariance test
    // is only evidence if the padding really reaches the pipeline, and the
    // invariant only holds because the store removes it before any pass can see
    // it — so assert the fold is identical either way AND name the mechanism,
    // instead of trusting that a green run means something.
    let asm = concat!(
        "f:\n",
        "    movq %rdi, %rcx\n",
        "    movq (%rcx), %rax\n",
        "    movq %rbx, %rcx\n",
        "    movq %rcx, -16(%rbp)\n",
        "    ret\n",
        ".size f, .-f\n",
    );
    let padded = pad(asm, "    ");

    // The padding is really there, on every line, and the line count is
    // unchanged (so the two inputs are the same assembly).
    assert_ne!(padded, asm);
    assert_eq!(split_lines(&padded).len(), split_lines(asm).len());
    assert!(
        split_lines(&padded)
            .iter()
            .all(|line| line.ends_with("    "))
    );

    // 1. Same optimization either way — the address copy is folded through.
    let plain_out = normalize(&peephole_optimize(asm.to_string()));
    let padded_out = normalize(&peephole_optimize(padded.clone()));
    assert_eq!(plain_out, padded_out, "the fold is whitespace-sensitive");
    assert!(
        plain_out
            .iter()
            .any(|line| line.contains("movq (%rdi), %rax")),
        "the fold this control depends on did not fire at all: {plain_out:?}"
    );

    // 2. The output is canonical: no line the pipeline emits carries trailing
    //    blanks, so padded and unpadded inputs produce byte-identical assembly.
    assert_eq!(
        peephole_optimize(asm.to_string()),
        peephole_optimize(padded.clone()),
        "the outputs differ as bytes; the store did not normalize the padding away"
    );

    // 3. The mechanism, at the boundary itself: the raw text has the blanks and
    //    the store does not hand them out.
    let store = LineStore::new(padded.clone());
    assert_eq!(store.len(), split_lines(&padded).len());
    for i in 0..store.len() {
        let line = store.get(i);
        assert_eq!(
            line,
            line.trim_end(),
            "LineStore::get handed a pass trailing whitespace: {line:?}"
        );
    }

    // 4. Pass-generated text obeys the same rule, so a later pass cannot see
    //    blanks an earlier one formatted into its replacement.
    let mut store = LineStore::new(asm.to_string());
    store.replace(1, "    movq %rax, %rbx   \t".to_string());
    assert_eq!(store.get(1), "    movq %rax, %rbx");
}

#[test]
fn a_padded_load_still_forwards_the_constant_store() {
    // Regression, named: this is the shape the corpus sweep found in lccc's own
    // sha256 output (`work/sha256_lccc.s`, an LCG loop).  One trailing space
    // after the load was enough to stop the store-to-load forward of the
    // `movq $0` above it, because the pass slices the slot operand out of the
    // raw line and the blank moved the slice.  The constant stayed in memory
    // and the compare re-read it every iteration.
    let asm = concat!(
        "f:\n",
        "    movq $0, 152(%rsp)\n",
        ".LBB25:\n",
        "    movq 152(%rsp), %rax\n",
        "    cmpl $131072, %eax\n",
        "    jb .LBB27\n",
        "    ret\n",
        ".LBB27:\n",
        "    ret\n",
        ".size f, .-f\n",
    );
    for padding in PADDINGS {
        let out = normalize(&peephole_optimize(pad(asm, padding)));
        assert!(
            out.iter()
                .any(|line| line.contains("movl $0, %eax") || line.contains("movq $0, %rax")),
            "the store of $0 was not forwarded into the padded load \
             (padding {padding:?}): {out:?}"
        );
        assert_eq!(
            out,
            normalize(&peephole_optimize(asm.to_string())),
            "forwarding depends on trailing whitespace (padding {padding:?})"
        );
    }
}

/// The operand space: every mnemonic whose destination the passes look up in a
/// register-name table, every spelling a destination can take in AT&T text (all
/// four GP widths including the high bytes, the vector and x87 files, segment
/// and control registers, and junk no recognizer should accept), and the sources
/// an operand pair can be built from.
const PANIC_MNEMONICS: &[&str] = &[
    "movb", "movw", "movl", "movq", "movzbl", "movzbw", "movzwl", "movsbw", "movswl", "movslq",
    "addb", "addw", "addl", "addq", "subl", "subq", "andl", "andq", "orl", "orq", "xorl", "xorq",
    "leal", "leaq", "testl", "testq", "cmpl", "cmpq", "imull", "imulq", "incl", "incq", "decl",
    "decq", "notl", "negq", "sarl", "shlq", "sete", "setne", "setl", "cmovel", "cmovneq", "popw",
    "popl", "popq", "pushw", "pushl", "pushq", "movd", "vmovd", "vmovq", "movaps", "pxor",
];
const PANIC_SPELLINGS: &[&str] = &[
    "%rax", "%rcx", "%rdx", "%rbx", "%rsp", "%rbp", "%rsi", "%rdi", "%r8", "%r15", "%eax", "%ecx",
    "%r15d", "%ax", "%cx", "%al", "%cl", "%ah", "%ch", "%dh", "%bh", "%xmm0", "%xmm15", "%ymm0",
    "%ymm31", "%mm0", "%st", "%st(0)", "%st(3)", "%fs", "%gs", "%cr0", "%dr0", "%eiz", "%zz", "%",
    "", "%k0",
];
const PANIC_SOURCES: &[&str] = &[
    "$0",
    "$1",
    "$-1",
    "$0xff",
    "$4294967296",
    "%rax",
    "%ecx",
    "%st",
];

/// The shapes that put a spelling in each operand position, including the
/// immediate-load / ALU pair that used to panic.
fn panic_shapes(mnemonic: &str, spelling: &str) -> [String; 7] {
    [
        format!("    {mnemonic} $1, {spelling}"),
        format!("    {mnemonic} {spelling}, {spelling}"),
        format!("    {mnemonic} %rax, {spelling}"),
        format!("    {mnemonic} {spelling}, %rax"),
        format!("    {mnemonic} ({spelling}), %rax"),
        format!("    {mnemonic} %rax, ({spelling})"),
        format!("    {mnemonic} -8(%rbp), {spelling}"),
    ]
}

/// Run one assembled shape and report a panic instead of aborting the test.
fn probe_shape(shape: &str, source: &str, mnemonic: &str, padding: &str) -> Option<String> {
    let asm = format!(
        "f:\n{shape}{padding}\n    addq {source}, %rax{padding}\n    {mnemonic} {source}, %rax{padding}\n    ret\n.size f, .-f\n"
    );
    match try_peephole(&asm) {
        Ok(_) => None,
        Err(e) => Some(format!("{e}\n  on input: {asm:?}")),
    }
}

fn report_panics(panics: &[String], checked: usize, floor: usize, shown: usize) {
    assert!(
        panics.is_empty(),
        "{} of {checked} operand spellings panicked the pipeline. First {shown}:\n{}",
        panics.len(),
        panics
            .iter()
            .take(shown)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        checked >= floor,
        "corpus too small to be evidence: {checked} (expected at least {floor})"
    );
}

#[test]
fn the_pipeline_never_panics_on_any_operand_spelling() {
    // Totality, not whitespace: `peephole_optimize` is a function from text to
    // text and must not panic on any of it.  It did — `movq $3, %xmm0` and
    // `movq $1, %st` both aborted the compiler with "index out of bounds: the
    // len is 16 but the index is 24/255", because the immediate-load/ALU pair
    // indexed the 16-entry GP name table with a family id that was an XMM
    // family (24) or REG_NONE (255) and checked validity only on the line AFTER
    // the index.  No padding required: the whitespace corpus merely made it
    // reachable often enough to notice.
    //
    // Enumerating the operand space is what finds the other sites of that class
    // (there are ~30 `REG_NAMES[..][fam as usize]` indexes in the tree, some
    // guarded by `fam <= REG_GP_MAX` and some not) without auditing each by
    // hand.  This is the fast subset — every spelling against the two shapes
    // that decide the destination lookup; the exhaustive cross product with
    // every source and padding is `..on_the_full_operand_matrix`, which the
    // regression gate runs.
    let mut checked = 0usize;
    let mut panics: Vec<String> = Vec::new();
    for mnemonic in PANIC_MNEMONICS {
        for spelling in PANIC_SPELLINGS {
            for shape in panic_shapes(mnemonic, spelling).iter().take(2) {
                checked += 1;
                panics.extend(probe_shape(shape, "$1", mnemonic, ""));
            }
        }
    }
    report_panics(&panics, checked, 3_000, 5);
}

#[test]
#[ignore = "exhaustive operand matrix: ~150k pipeline runs, ~20s. Run by tests/regression/check_peephole_whitespace.sh"]
fn the_pipeline_never_panics_on_the_full_operand_matrix() {
    let mut checked = 0usize;
    let mut panics: Vec<String> = Vec::new();
    for mnemonic in PANIC_MNEMONICS {
        for spelling in PANIC_SPELLINGS {
            for shape in panic_shapes(mnemonic, spelling) {
                for source in PANIC_SOURCES {
                    for padding in ["", "    ", "\t"] {
                        checked += 1;
                        panics.extend(probe_shape(&shape, source, mnemonic, padding));
                    }
                }
            }
        }
    }
    report_panics(&panics, checked, 100_000, 5);
}

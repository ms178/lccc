//! Legacy-SSE → VEX promotion inside functions that use 256-bit registers.
//!
//! # Why
//!
//! The vectoriser emits its loop bodies in VEX-256 (`vmulps … %ymm`), but the
//! scalar remainder, the reduction epilogue and every other scalar FP
//! instruction in the same function still come out of the text emitters in
//! *legacy* SSE encoding (`movss`, `cvtsi2sdq`, `ucomisd`, …).  The function
//! runs its `vzeroupper` only in the epilogue, so that scalar code executes
//! with **dirty upper YMM state**.  Measured consequences (Intel ORM §15.3,
//! Agner Fog "Mixing AVX and SSE"; uops.info):
//!
//! * Skylake and every later P-core (SKL/ICL/GLC/RPC, i.e. the Raptor Lake
//!   target): each legacy-SSE instruction that writes an XMM register gets a
//!   merge µop and a *false dependency on the previous value of that
//!   register*.  A remainder loop `movss (%rdi),%xmm4; vmulss …,%xmm4` thus
//!   serialises on `xmm4` at FP latency (≈5 cycles/element instead of 1).
//! * Sandy/Ivy Bridge, Haswell, Broadwell: a state transition of ~70 cycles
//!   on *every* legacy→VEX and VEX→legacy boundary — a mixed remainder loop
//!   pays two per iteration.
//! * Zen: no penalty (documented; the rewrite is still a no-op there).
//!
//! GCC and Clang avoid this by encoding *everything* VEX once `-mavx` is on.
//! lccc has ~60 scalar-FP emission sites in six files; rewriting them one
//! by one is neither robust nor reviewable, and the peephole framework's
//! own `fuse_mov_scalar_fp_into_vex_op` already depends on seeing the
//! legacy spelling.  This pass therefore runs **last**, on the final text.
//!
//! # Soundness
//!
//! A legacy SSE instruction with an XMM destination leaves bits 255:128 of
//! the register untouched; its VEX form zeroes them.  The rewrite is exact
//! for bits 127:0 in every case, so the only observable difference is the
//! upper half, and that is only observable if the function *reads*
//! `%ymmN` after writing `%xmmN` legacy-style.  Rules:
//!
//! 1. Only functions that mention `%ymm` at all are touched (no 256-bit use
//!    → no dirty state → legacy encodings are shorter and cost nothing).
//! 2. An instruction whose destination is `%xmmN` is rewritten only if no
//!    *control-flow-reachable* later line reads `%ymmN`.  Reachability is
//!    computed over the emitted text: labels, fall-through, `jmp`/`jcc`
//!    targets; an unresolved or indirect branch conservatively reaches
//!    every line.  Reads beyond a line that **fully overwrites** the upper
//!    half (a VEX instruction with `%ymmN`/`%xmmN` as its destination, or
//!    `vzeroupper`) do not count: the legacy merge would be dead.  (The
//!    ABI makes the upper halves undefined across calls and at function
//!    entry/return, so external readers do not count either.)
//! 3. Instructions without an XMM destination (stores, compares, GPR
//!    destinations) are always exact and always rewritten.
//! 4. Functions containing inline asm (`#APP`) are skipped wholesale.
//! 5. Register-to-register `movss`/`movsd` keep their merge semantics via
//!    the three-operand VEX form `vmovss %a, %b, %b` (dst = b[127:32] ∪
//!    a[31:0]), identical to the legacy instruction for bits 127:0.
//! 6. Register-to-register `movd`/`movq` are **never** rewritten: their
//!    VEX forms zero dst[127:32] / dst[127:64] where the legacy encodings
//!    merge, and no three-operand spelling exists (both GAS and lccc's
//!    integrated assembler reject `vmovd %a, %b, %b`).  GPR-source and
//!    memory forms are zero-extending in BOTH encodings and are rewritten.
//! 7. Every VEX mnemonic emitted here was assembled by lccc's integrated
//!    assembler and compared byte-for-byte against GAS (see the unit tests
//!    and `tests/regression/vex_promote_remainder.c`).
//!
//! Anything the table does not list is left alone; an unknown legacy
//! instruction merely keeps its (already correct) encoding.

/// How the operand list changes between the legacy and the VEX spelling.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Form {
    /// Same operand list (`movdqu m, x` → `vmovdqu m, x`; `pshufd $i, a, b`
    /// → `vpshufd $i, a, b`; GPR/flags destinations).
    Two,
    /// Destructive two-address op → non-destructive three-address with the
    /// destination duplicated as the first source (`addss a, b` →
    /// `vaddss a, b, b`; `shufps $i, a, b` → `vshufps $i, a, b, b`).
    Three,
    /// `movss`/`movsd`: `Two` for memory forms, `Three` register-to-register.
    MovScalar,
    /// `cvt(t)s[sd]2si[lq]`: GPR destination; the operand-size suffix is
    /// dropped because the VEX spelling takes the width from the GPR.
    CvtToInt,
    /// `cvtsi2s[sd][lq]`: `Three`; an unsuffixed spelling gets its suffix
    /// from the GPR name so the assembler never has to guess.
    CvtFromInt,
}

/// Legacy mnemonic → operand form.  The VEX mnemonic is `v` + legacy.
fn lookup(mnemonic: &str) -> Option<Form> {
    use Form::*;
    Some(match mnemonic {
        "movss" | "movsd" => MovScalar,
        "cvttss2si" | "cvttss2sil" | "cvttss2siq" | "cvttsd2si" | "cvttsd2sil"
        | "cvttsd2siq" | "cvtss2si" | "cvtss2sil" | "cvtss2siq" | "cvtsd2si" | "cvtsd2sil"
        | "cvtsd2siq" => CvtToInt,
        "cvtsi2ss" | "cvtsi2ssl" | "cvtsi2ssq" | "cvtsi2sd" | "cvtsi2sdl" | "cvtsi2sdq" => {
            CvtFromInt
        }
        // Moves, shuffles with immediate, conversions, extracts, flag
        // writers: operand list unchanged.  `movd`/`movq` are handled
        // separately: their register-to-register form has no safe VEX
        // spelling (merge → zero, no three-operand encoding exists).
        "movdqu" | "movdqa" | "movaps" | "movups" | "movapd" | "movupd"
        | "lddqu" | "movddup" | "movshdup" | "movsldup" | "pshufd" | "pshufhw" | "pshuflw"
        | "cvtdq2ps" | "cvtps2pd" | "cvtpd2ps" | "cvttps2dq" | "cvtps2dq" | "cvtdq2pd"
        | "cvttpd2dq" | "cvtpd2dq" | "ptest" | "ucomiss" | "ucomisd" | "comiss" | "comisd"
        | "pextrb" | "pextrw" | "pextrd" | "pextrq" | "pmovmskb" | "movmskps" | "movmskpd"
        | "pabsb" | "pabsw" | "pabsd" | "sqrtps" | "sqrtpd" | "rcpps" | "rsqrtps"
        | "roundps" | "roundpd" | "pmovzxbw" | "pmovzxbd" | "pmovzxbq" | "pmovzxwd"
        | "pmovzxwq" | "pmovzxdq" | "pmovsxbw" | "pmovsxbd" | "pmovsxbq" | "pmovsxwd"
        | "pmovsxwq" | "pmovsxdq" | "phminposuw" | "aesimc" | "movntdq" | "movntdqa"
        | "movntps" | "movntpd" | "aeskeygenassist" => Two,
        // Destructive binary ops: destination becomes the first source.
        "addss" | "addsd" | "subss" | "subsd" | "mulss" | "mulsd" | "divss" | "divsd"
        | "minss" | "minsd" | "maxss" | "maxsd" | "sqrtss" | "sqrtsd" | "rcpss" | "rsqrtss"
        | "cvtss2sd" | "cvtsd2ss" | "addps" | "addpd" | "subps" | "subpd" | "mulps"
        | "mulpd" | "divps" | "divpd" | "minps" | "minpd" | "maxps" | "maxpd" | "xorps"
        | "xorpd" | "andps" | "andpd" | "andnps" | "andnpd" | "orps" | "orpd" | "pxor"
        | "pand" | "pandn" | "por" | "paddb" | "paddw" | "paddd" | "paddq" | "psubb"
        | "psubw" | "psubd" | "psubq" | "paddusb" | "paddusw" | "paddsb" | "paddsw"
        | "psubusb" | "psubusw" | "psubsb" | "psubsw" | "pminsw" | "pmaxsw" | "pminub"
        | "pmaxub" | "pminsb" | "pmaxsb" | "pminsd" | "pmaxsd" | "pminud" | "pmaxud"
        | "pminuw" | "pmaxuw" | "pcmpeqb" | "pcmpeqw" | "pcmpeqd" | "pcmpeqq" | "pcmpgtb"
        | "pcmpgtw" | "pcmpgtd" | "pcmpgtq" | "punpcklbw" | "punpcklwd" | "punpckldq"
        | "punpcklqdq" | "punpckhbw" | "punpckhwd" | "punpckhdq" | "punpckhqdq"
        | "packsswb" | "packssdw" | "packuswb" | "packusdw" | "pmulld" | "pmullw"
        | "pmulhw" | "pmulhuw" | "pmuludq" | "pmuldq" | "pmaddwd" | "pmaddubsw" | "psadbw"
        | "pavgb" | "pavgw" | "pshufb" | "palignr" | "pclmulqdq" | "aesenc" | "aesenclast"
        | "aesdec" | "aesdeclast" | "pinsrb" | "pinsrw" | "pinsrd" | "pinsrq" | "shufps"
        | "shufpd" | "unpcklps" | "unpckhps" | "unpcklpd" | "unpckhpd" | "movhlps"
        | "movlhps" | "roundss" | "roundsd" | "psllw" | "pslld" | "psllq" | "psrlw"
        | "psrld" | "psrlq" | "psraw" | "psrad" | "pslldq" | "psrldq" | "addsubps"
        | "addsubpd" | "haddps" | "haddpd" | "hsubps" | "hsubpd" | "blendps" | "blendpd"
        | "pblendw" | "insertps" | "dpps" | "dppd" | "mpsadbw" | "cmpps" | "cmppd" | "cmpss"
        | "cmpsd" | "cmpeqss" | "cmpltss" | "cmpless" | "cmpunordss" | "cmpneqss"
        | "cmpnltss" | "cmpnless" | "cmpordss" | "cmpeqsd" | "cmpltsd" | "cmplesd"
        | "cmpunordsd" | "cmpneqsd" | "cmpnltsd" | "cmpnlesd" | "cmpordsd" | "cmpeqps"
        | "cmpltps" | "cmpleps" | "cmpunordps" | "cmpneqps" | "cmpnltps" | "cmpnleps"
        | "cmpordps" | "cmpeqpd" | "cmpltpd" | "cmplepd" | "cmpunordpd" | "cmpneqpd"
        | "cmpnltpd" | "cmpnlepd" | "cmpordpd" => Three,
        _ => return None,
    })
}

/// Split an AT&T operand list at top-level commas (parentheses of a memory
/// operand are never split).
fn split_operands(s: &str) -> Vec<&str> {
    let mut out = Vec::with_capacity(3);
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, b) in s.bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        out.push(last);
    }
    out
}

/// `%xmmN` → `N` (0..=15); `None` for anything else.
fn xmm_index(op: &str) -> Option<u32> {
    let rest = op.strip_prefix("%xmm")?;
    let n: u32 = rest.parse().ok()?;
    (n < 16).then_some(n)
}

/// Bitmask of every `%ymmN` mentioned in `text`.
fn ymm_mask(text: &str) -> u32 {
    let mut mask = 0u32;
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(pos) = text[i..].find("%ymm") {
        let mut j = i + pos + 4;
        let mut n = 0u32;
        let mut digits = 0;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            n = n * 10 + (bytes[j] - b'0') as u32;
            j += 1;
            digits += 1;
        }
        if digits > 0 && n < 16 {
            mask |= 1 << n;
        }
        i = j.max(i + pos + 4);
    }
    mask
}

fn is_gpr32(op: &str) -> bool {
    op.starts_with("%e")
        || matches!(
            op,
            "%r8d" | "%r9d" | "%r10d" | "%r11d" | "%r12d" | "%r13d" | "%r14d" | "%r15d"
        )
}

/// Rewrite one instruction line (already known to be an instruction, not a
/// label/directive/comment).  `allowed` is the bitmask of YMM aliases whose
/// upper half cannot be observed after this line (no control-flow-reachable
/// later read).  Returns the new text or `None` to keep it.
fn rewrite_line(line: &str, allowed: u32) -> Option<String> {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    // Lines with comments are left alone: the emitter's annotated lines are
    // rare and never in hot scalar tails.
    if trimmed.contains('#') || !trimmed.contains("%xmm") {
        return None;
    }
    let (mnemonic, rest) = match trimmed.find(|c: char| c.is_ascii_whitespace()) {
        Some(p) => (&trimmed[..p], trimmed[p..].trim()),
        None => return None,
    };
    if mnemonic.starts_with('v') {
        return None;
    }
    let ops = split_operands(rest);
    if ops.is_empty() || ops.len() > 3 {
        return None;
    }
    // Register-to-register movd/movq: legacy merges dst[127:32] / dst[127:64],
    // every VEX spelling zeroes them, and no three-operand encoding exists.
    // Never rewrite.  (GPR/memory forms fall through: both encodings zero.)
    if matches!(mnemonic, "movd" | "movq")
        && ops.len() == 2
        && ops.iter().all(|o| o.starts_with("%xmm"))
    {
        return None;
    }
    // SSE2 `pextrw %xmm, %r32` has no immediate and no VEX spelling; only
    // the SSE4.1 `pextrw $imm, ...` form is promotable.
    if mnemonic == "pextrw" && !ops[0].starts_with('$') {
        return None;
    }
    // movd/movq GPR-source and memory forms: zero-extending in both
    // encodings, two-operand VEX spelling, operand list unchanged.
    let form = if matches!(mnemonic, "movd" | "movq") {
        Some(Form::Two)
    } else {
        lookup(mnemonic)
    }?;
    let dst = *ops.last().unwrap();
    // Rule 2: an XMM destination whose YMM alias is read on any
    // control-flow-reachable later line must keep the upper-half-preserving
    // legacy encoding.
    if let Some(n) = xmm_index(dst) {
        if allowed & (1 << n) == 0 {
            return None;
        }
    }
    // Reject anything unusual defensively (masking syntax, x87 operands).
    if ops
        .iter()
        .any(|o| o.is_empty() || o.contains('{') || o.contains("%st"))
    {
        return None;
    }
    let reg_reg = ops.len() == 2 && ops.iter().all(|o| o.starts_with("%xmm"));
    let (vmn, three) = match form {
        Form::Two => (format!("v{mnemonic}"), false),
        Form::Three => (format!("v{mnemonic}"), true),
        Form::MovScalar => (format!("v{mnemonic}"), reg_reg),
        Form::CvtToInt => {
            if xmm_index(dst).is_some() {
                return None; // malformed for this form
            }
            // Drop the `l`/`q` suffix; the GPR operand carries the width.
            let base = mnemonic
                .strip_suffix('q')
                .or_else(|| mnemonic.strip_suffix('l'))
                .unwrap_or(mnemonic);
            (format!("v{base}"), false)
        }
        Form::CvtFromInt => {
            let src = ops.first().copied().unwrap_or("");
            let suffixed = mnemonic.ends_with('l') || mnemonic.ends_with('q');
            let mn = if suffixed {
                mnemonic.to_string()
            } else if is_gpr32(src) {
                format!("{mnemonic}l")
            } else if src.starts_with("%r") {
                format!("{mnemonic}q")
            } else {
                return None; // memory source without suffix: width unknown
            };
            (format!("v{mn}"), true)
        }
    };
    if three && xmm_index(dst).is_none() {
        // Three-operand forms need an XMM destination to duplicate.
        return None;
    }
    let mut out = String::with_capacity(line.len() + 12);
    out.push_str(indent);
    out.push_str(&vmn);
    out.push(' ');
    for (i, op) in ops.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(op);
    }
    if three {
        out.push_str(", ");
        out.push_str(dst);
    }
    Some(out)
}

fn is_function_label(line: &str) -> bool {
    let t = line.trim_end();
    if !t.ends_with(':') || t.starts_with(".L") || t.starts_with(' ') || t.starts_with('\t') {
        return false;
    }
    let name = &t[..t.len() - 1];
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'$' || b == b'@')
}

/// Instruction line? (not a label, directive, comment, or blank).
fn is_insn_line(t: &str) -> bool {
    !t.is_empty() && !t.starts_with('.') && !t.ends_with(':') && !t.starts_with('#')
}

/// A label line (`foo:` / `.LBB2:`) → its name, else `None`.
fn label_name(l: &str) -> Option<&str> {
    let t = l.trim_end();
    if !t.ends_with(':') || t.is_empty() || t.contains(' ') || t.contains('\t') {
        return None;
    }
    Some(&t[..t.len() - 1])
}

/// Successor line indices of `i` inside one function body.  Direct `jmp`/
/// `jcc` targets resolve through `labels`; an unresolved or indirect branch
/// conservatively reaches every line (fail closed).
fn successor_lines(body: &[&str], labels: &std::collections::HashMap<&str, usize>, i: usize) -> Vec<usize> {
    let n = body.len();
    let fall = |v: &mut Vec<usize>| {
        if i + 1 < n {
            v.push(i + 1);
        }
    };
    let t = body[i].trim();
    if !is_insn_line(t) {
        let mut v = Vec::new();
        fall(&mut v);
        return v;
    }
    let (mn, rest) = match t.find(|c: char| c.is_ascii_whitespace()) {
        Some(p) => (&t[..p], t[p..].trim()),
        None => (t, ""),
    };
    match mn {
        "ret" | "retq" | "iret" | "iretq" | "ud2" => Vec::new(),
        "jmp" | "jmpq" => {
            if rest.starts_with('*') {
                (0..n).collect()
            } else if let Some(&j) = labels.get(rest) {
                vec![j]
            } else {
                (0..n).collect()
            }
        }
        _ if mn.len() > 1 && mn.starts_with('j') => {
            let mut v = Vec::new();
            fall(&mut v);
            if rest.starts_with('*') {
                v.extend(0..n);
            } else if let Some(&j) = labels.get(rest) {
                v.push(j);
            } else {
                v.extend(0..n);
            }
            v
        }
        _ => {
            let mut v = Vec::new();
            fall(&mut v);
            v
        }
    }
}

/// Per-line promotion masks for one function body: `allowed[i]` has bit N
/// set iff no read of `%ymmN` is control-flow-reachable from line i.
/// Rule 2 of the soundness contract; reads include any instruction line
/// mentioning `%ymmN` (write-only mentions are deliberately counted too —
/// conservative, no precision loss in practice).
fn allowed_masks(body: &[&str]) -> Vec<u32> {
    let n = body.len();
    let mut labels = std::collections::HashMap::new();
    for (i, l) in body.iter().enumerate() {
        if let Some(name) = label_name(l) {
            labels.entry(name).or_insert(i);
        }
    }
    // Read lines per alias.  A `%ymmN` occurrence counts as a READ unless it
    // sits in the destination (last-operand) position of the instruction —
    // every VEX-256 op writes its YMM destination in full, so `vpbroadcastd
    // %xmm0, %ymm0` must not block promotion of the `movd` that feeds it
    // (the merge the legacy encoding preserves is dead).  A `%ymmN` in any
    // source position observes the upper half and counts.
    let mut read_lines: [Vec<usize>; 16] = std::array::from_fn(|_| Vec::new());
    for (i, l) in body.iter().enumerate() {
        let t = l.trim();
        if is_insn_line(t) {
            // Span of the final operand (AT&T: the destination).  The
            // trimmed line ends with it, so the span is derivable from the
            // operand's length.
            let rest = match t.find(|c: char| c.is_ascii_whitespace()) {
                Some(p) => t[p..].trim(),
                None => "",
            };
            let last_span = split_operands(rest).last().map(|op| (t.len() - op.len(), t.len()));
            if let Some(rest) = t.find("%ymm") {
                let mut j = rest + 4;
                while j < t.len() {
                    let b = t.as_bytes()[j];
                    if b.is_ascii_digit() {
                        let mut k = j;
                        let mut num = 0usize;
                        while k < t.len() && t.as_bytes()[k].is_ascii_digit() {
                            num = num * 10 + (t.as_bytes()[k] - b'0') as usize;
                            k += 1;
                        }
                        // Position of the '%' of this occurrence.
                        let occ_start = j - 4;
                        let in_dst = last_span.is_some_and(|(a, _)| occ_start >= a);
                        if num < 16 && !in_dst {
                            read_lines[num].push(i);
                        }
                        j = k;
                    } else {
                        j += 1;
                    }
                }
            }
        }
    }
    // Kill mask per line: bit N set when the instruction fully overwrites
    // the upper half of alias N (VEX/`vzeroupper`; EVEX forms — zmm
    // operands, masking syntax, EVEX-only mnemonics with digit suffixes —
    // deliberately never count: EVEX preserves the upper bits of XMM
    // destinations).
    let mut kill: Vec<u32> = vec![0; n];
    for (i, l) in body.iter().enumerate() {
        let t = l.trim();
        if !is_insn_line(t) {
            continue;
        }
        let (mn, rest) = match t.find(|c: char| c.is_ascii_whitespace()) {
            Some(p) => (&t[..p], t[p..].trim()),
            None => (t, ""),
        };
        if mn == "vzeroupper" {
            // Kills all 16 YMM aliases (the 0xFFFF mask, spelled for the
            // per-alias bit width).
            kill[i] = 0xFFFF;
            continue;
        }
        if !mn.starts_with('v') || t.contains('{') || t.contains("%zmm") {
            continue;
        }
        // EVEX-only encodings carry a width digit in the mnemonic
        // (vmovdqu64, vpternlogd is EVEX-only too but never emitted; the
        // digit rule covers every EVEX mnemonic lccc can produce).
        if mn.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        let last = split_operands(rest).last().copied().unwrap_or("");
        if let Some(num) = last.strip_prefix("%ymm").and_then(|r| r.parse::<u32>().ok()) {
            if num < 16 {
                kill[i] |= 1 << num;
            }
        } else if let Some(num) = last.strip_prefix("%xmm").and_then(|r| r.parse::<u32>().ok()) {
            if num < 16 {
                kill[i] |= 1 << num;
            }
        }
    }
    // Reverse reachability with kill absorption: blocked[a][i] = line i can
    // reach a read of alias a without crossing a full overwrite of it.  A
    // kill-only line absorbs the walk (its predecessors cannot be observed
    // through it); a line that reads the alias always propagates (its own
    // read observes whatever the predecessors left in the upper half).
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 0..n {
        for &succ in &successor_lines(body, &labels, i) {
            preds[succ].push(i);
        }
    }
    let mut allowed = vec![u32::MAX; n];
    for a in 0..16 {
        if read_lines[a].is_empty() {
            continue;
        }
        let read_set: std::collections::HashSet<usize> = read_lines[a].iter().copied().collect();
        let mut blocked = vec![false; n];
        let mut stack = read_lines[a].clone();
        for &i in &stack {
            blocked[i] = true;
        }
        while let Some(i) = stack.pop() {
            if kill[i] & (1 << a) != 0 && !read_set.contains(&i) {
                continue;
            }
            for &p in &preds[i] {
                if !blocked[p] {
                    blocked[p] = true;
                    stack.push(p);
                }
            }
        }
        for (i, b) in blocked.iter().enumerate() {
            if *b {
                allowed[i] &= !(1 << a);
            }
        }
    }
    allowed
}

/// Promote legacy SSE to VEX in every function that uses `%ymm`.  Returns
/// the number of rewritten lines.
pub fn promote_legacy_sse_to_vex(asm: &mut String) -> usize {
    let lines: Vec<&str> = asm.split_inclusive('\n').collect();
    // Function ranges: [start, end) line indices.
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut cur = 0usize;
    for (i, l) in lines.iter().enumerate() {
        if is_function_label(l) {
            if i > cur {
                ranges.push((cur, i));
            }
            cur = i;
        }
    }
    if lines.len() > cur {
        ranges.push((cur, lines.len()));
    }

    let mut out = String::with_capacity(asm.len() + 1024);
    let mut rewritten = 0usize;
    for (start, end) in ranges {
        let body = &lines[start..end];
        let mut ymm_used = 0u32;
        let mut has_asm = false;
        for l in body {
            if l.contains("%ymm") {
                ymm_used |= ymm_mask(l);
            }
            if l.trim() == "#APP" {
                has_asm = true;
            }
        }
        if ymm_used == 0 || has_asm {
            for l in body {
                out.push_str(l);
            }
            continue;
        }
        let allowed = allowed_masks(body);
        for (idx, l) in body.iter().enumerate() {
            let content = l.strip_suffix('\n').unwrap_or(l);
            let t = content.trim_start();
            if is_insn_line(t) {
                if let Some(new) = rewrite_line(content, allowed[idx]) {
                    out.push_str(&new);
                    if l.ends_with('\n') {
                        out.push('\n');
                    }
                    rewritten += 1;
                    continue;
                }
            }
            out.push_str(l);
        }
    }
    if rewritten > 0 {
        *asm = out;
    }
    rewritten
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> (String, usize) {
        let mut s = src.to_string();
        let n = promote_legacy_sse_to_vex(&mut s);
        (s, n)
    }

    #[test]
    fn scalar_tail_after_ymm_loop_is_promoted() {
        let src = "\
f:
\tvmulps (%rsi,%r10), %ymm2, %ymm0
\tvmovups %ymm0, (%rdi,%r10)
\tmovss .LCFP_0(%rip), %xmm3
\tmovss (%rdi), %xmm4
\tvmulss %xmm3, %xmm4, %xmm4
\tmovss %xmm4, (%r10)
\tcvtsi2sdq %rax, %xmm5
\tcvttsd2siq %xmm5, %rax
\tucomisd %xmm5, %xmm6
\taddsd %xmm5, %xmm6
\tmovsd %xmm6, %xmm7
\tvzeroupper
\tret
";
        let (out, n) = run(src);
        assert_eq!(n, 8, "{out}");
        assert!(out.contains("\tvmovss .LCFP_0(%rip), %xmm3\n"));
        assert!(out.contains("\tvmovss (%rdi), %xmm4\n"));
        assert!(out.contains("\tvmovss %xmm4, (%r10)\n"));
        assert!(out.contains("\tvcvtsi2sdq %rax, %xmm5, %xmm5\n"));
        assert!(out.contains("\tvcvttsd2si %xmm5, %rax\n"));
        assert!(out.contains("\tvucomisd %xmm5, %xmm6\n"));
        assert!(out.contains("\tvaddsd %xmm5, %xmm6, %xmm6\n"));
        assert!(out.contains("\tvmovsd %xmm6, %xmm7, %xmm7\n"));
        // Already-VEX lines are untouched.
        assert!(out.contains("\tvmulss %xmm3, %xmm4, %xmm4\n"));
    }

    #[test]
    fn alias_only_written_earlier_is_still_promoted() {
        // ymm0/ymm2 are WRITTEN at the top but never read afterwards, so
        // the scalar writes below may zero the upper half without an
        // observer.  (The old whole-function gate kept these legacy.)
        let src = "\
g:
\tvmovdqa %ymm0, %ymm2
\tmovss (%rdi), %xmm2
\tmovss (%rdi), %xmm0
\tmovss %xmm2, (%rsi)
\tpaddd %xmm1, %xmm2
\tpaddd %xmm1, %xmm3
\tret
";
        let (out, n) = run(src);
        assert_eq!(n, 5, "{out}");
        assert!(out.contains("\tvmovss (%rdi), %xmm2\n"));
        assert!(out.contains("\tvmovss (%rdi), %xmm0\n"));
        assert!(out.contains("\tvmovss %xmm2, (%rsi)\n"));
        assert!(out.contains("\tvpaddd %xmm1, %xmm2, %xmm2\n"));
        assert!(out.contains("\tvpaddd %xmm1, %xmm3, %xmm3\n"));
    }

    #[test]
    fn alias_read_later_stays_legacy() {
        // The scalar xmm2 write is followed by a read of %ymm2 → the upper
        // half is observable → must keep the legacy encoding.
        let src = "\
g:
\tmovss (%rdi), %xmm2
\tvaddps %ymm2, %ymm0, %ymm0
\tret
";
        let (out, n) = run(src);
        assert_eq!(n, 0, "{out}");
        assert!(out.contains("\tmovss (%rdi), %xmm2\n"));
    }

    #[test]
    fn alias_read_behind_backward_branch_stays_legacy() {
        // The loop head reads %ymm2; a conditional back-edge reaches it from
        // after the scalar write → still observable → legacy.
        let src = "\
g:
.Ltop:
\tvaddps %ymm2, %ymm0, %ymm0
\tmovss (%rdi), %xmm2
\tdecl %eax
\tjne .Ltop
\tret
";
        let (out, n) = run(src);
        assert_eq!(n, 0, "{out}");
        assert!(out.contains("\tmovss (%rdi), %xmm2\n"));
    }

    #[test]
    fn alias_read_only_on_cold_path_still_blocks() {
        // The read of %ymm2 sits behind a conditional branch: some paths
        // reach it → conservative rule keeps the write legacy.
        let src = "\
g:
\tmovss (%rdi), %xmm2
\tcmpq %rax, %rbx
\tje .Lcold
\tret
.Lcold:
\tvaddps %ymm2, %ymm0, %ymm0
\tret
";
        let (out, n) = run(src);
        assert_eq!(n, 0, "{out}");
        assert!(out.contains("\tmovss (%rdi), %xmm2\n"));
    }

    #[test]
    fn alias_read_before_write_does_not_block() {
        // %ymm2 is read, then the function diverges from the vector loop
        // into a scalar tail that never reaches another ymm2 read: the tail
        // writes promote.  (This is the whole point of the refined gate —
        // the common "vector loop + scalar remainder" layout.)
        let src = "\
g:
\tvxorps %ymm2, %ymm2, %ymm2
.Lloop:
\tvaddps (%rsi), %ymm2, %ymm2
\taddq $32, %rsi
\tcmpq %rdi, %rsi
\tjb .Lloop
\tmovss (%r8), %xmm2
\tmovss %xmm2, (%r9)
\tret
";
        let (out, n) = run(src);
        assert_eq!(n, 2, "{out}");
        assert!(out.contains("\tvmovss (%r8), %xmm2\n"));
        assert!(out.contains("\tvmovss %xmm2, (%r9)\n"));
    }

    #[test]
    fn indirect_jump_blocks_promotion_fail_closed() {
        let src = "\
g:
\tvxorps %ymm3, %ymm3, %ymm3
\tmovss (%rdi), %xmm3
\tjmp *%rax
";
        let (out, n) = run(src);
        assert_eq!(n, 0, "{out}");
        assert!(out.contains("\tmovss (%rdi), %xmm3\n"));
    }

    #[test]
    fn movd_movq_regreg_are_never_promoted() {
        // No three-operand VEX spelling exists; GAS and the integrated
        // assembler reject vmovd/vmovq xmm,xmm,xmm.  Bits 127:32 / 127:64
        // would be zeroed where legacy merges.
        let src = "\
g:
\tvxorps %ymm1, %ymm1, %ymm1
\tmovd %xmm0, %xmm1
\tmovq %xmm2, %xmm1
\tret
";
        let (out, n) = run(src);
        assert_eq!(n, 0, "{out}");
        assert!(out.contains("\tmovd %xmm0, %xmm1\n"));
        assert!(out.contains("\tmovq %xmm2, %xmm1\n"));
    }

    #[test]
    fn movd_movq_gpr_and_memory_forms_are_promoted() {
        // Zero-extending in both encodings → always exact.
        let src = "\
g:
\tvxorps %ymm2, %ymm2, %ymm2
\tmovd %eax, %xmm1
\tmovq %rax, %xmm1
\tmovq (%rdi), %xmm1
\tmovd %xmm1, %eax
\tmovq %xmm1, %rax
\tret
";
        let (out, n) = run(src);
        assert_eq!(n, 5, "{out}");
        assert!(out.contains("\tvmovd %eax, %xmm1\n"));
        assert!(out.contains("\tvmovq %rax, %xmm1\n"));
        assert!(out.contains("\tvmovq (%rdi), %xmm1\n"));
        assert!(out.contains("\tvmovd %xmm1, %eax\n"));
        assert!(out.contains("\tvmovq %xmm1, %rax\n"));
    }

    #[test]
    fn pextrw_without_immediate_is_never_promoted() {
        // SSE2 pextrw %xmm, %r32 has no VEX spelling (vpextrw requires the
        // SSE4.1 immediate form).
        let src = "\
g:
\tvxorps %ymm3, %ymm3, %ymm3
\tpextrw %xmm3, %eax
\tpextrw $1, %xmm3, %eax
\tret
";
        let (out, n) = run(src);
        assert_eq!(n, 1, "{out}");
        assert!(out.contains("\tpextrw %xmm3, %eax\n"));
        assert!(out.contains("\tvpextrw $1, %xmm3, %eax\n"));
    }

    #[test]
    fn nontemporal_moves_are_promoted() {
        let src = "\
g:
\tvxorps %ymm4, %ymm4, %ymm4
\tmovntdq %xmm4, (%rdi)
\tmovntps %xmm4, (%rdi)
\tmovntdqa (%rsi), %xmm5
\tret
";
        let (out, n) = run(src);
        assert_eq!(n, 3, "{out}");
        assert!(out.contains("\tvmovntdq %xmm4, (%rdi)\n"));
        assert!(out.contains("\tvmovntps %xmm4, (%rdi)\n"));
        assert!(out.contains("\tvmovntdqa (%rsi), %xmm5\n"));
    }

    #[test]
    fn functions_without_ymm_or_with_inline_asm_are_untouched() {
        let plain = "h:\n\tmovss (%rdi), %xmm0\n\taddss %xmm1, %xmm0\n\tret\n";
        let (out, n) = run(plain);
        assert_eq!(n, 0);
        assert_eq!(out, plain);
        let asm =
            "k:\n\tvpxor %ymm0, %ymm0, %ymm0\n#APP\n\tnop\n#NO_APP\n\tmovss (%rdi), %xmm1\n\tret\n";
        let (out, n) = run(asm);
        assert_eq!(n, 0);
        assert_eq!(out, asm);
    }

    #[test]
    fn per_function_scoping_and_immediate_forms() {
        let src = "\
a:
\tvxorps %ymm0, %ymm0, %ymm0
\tpshufd $85, %xmm1, %xmm2
\tshufps $27, %xmm1, %xmm2
\tpinsrd $1, %eax, %xmm2
\tpextrd $1, %xmm2, %eax
\tpsllq $3, %xmm2
\tcvtsi2sd %eax, %xmm2
\tcvtsi2sd %rax, %xmm2
\tcvtsi2sd (%rdi), %xmm2
\tret
b:
\tmovss (%rdi), %xmm0
\tret
";
        let (out, n) = run(src);
        assert_eq!(n, 7, "{out}");
        assert!(out.contains("\tvpshufd $85, %xmm1, %xmm2\n"));
        assert!(out.contains("\tvshufps $27, %xmm1, %xmm2, %xmm2\n"));
        assert!(out.contains("\tvpinsrd $1, %eax, %xmm2, %xmm2\n"));
        assert!(out.contains("\tvpextrd $1, %xmm2, %eax\n"));
        assert!(out.contains("\tvpsllq $3, %xmm2, %xmm2\n"));
        assert!(out.contains("\tvcvtsi2sdl %eax, %xmm2, %xmm2\n"));
        assert!(out.contains("\tvcvtsi2sdq %rax, %xmm2, %xmm2\n"));
        // Unsuffixed memory-source conversion: width unknown → kept.
        assert!(out.contains("\tcvtsi2sd (%rdi), %xmm2\n"));
        // Function `b` has no ymm use → untouched.
        assert!(out.contains("b:\n\tmovss (%rdi), %xmm0\n"));
    }

    #[test]
    fn string_cmpsd_and_comments_are_never_touched() {
        let src =
            "c:\n\tvpxor %ymm0, %ymm0, %ymm0\n\trep cmpsd\n\tmovss (%rdi), %xmm1 # note\n\tret\n";
        let (out, n) = run(src);
        assert_eq!(n, 0);
        assert_eq!(out, src);
    }

    #[test]
    fn ymm_mask_parses_all_registers() {
        assert_eq!(ymm_mask("vaddps %ymm15, %ymm1, %ymm0"), (1 << 15) | (1 << 1) | 1);
        assert_eq!(ymm_mask("vmovups %ymm7, (%rdi)"), 1 << 7);
        assert_eq!(ymm_mask("nothing"), 0);
    }

    #[test]
    fn full_write_between_write_and_read_absorbs() {
        // The scalar movd feeds a broadcast that fully overwrites %ymm0
        // before the loop reads it: the legacy merge is dead, so the movd
        // promotes.  (The canonical scalar→broadcast→vector-loop layout.)
        let src = "\
g:
\tmovd %eax, %xmm0
\tvpbroadcastd %xmm0, %ymm0
.Lloop:
\tvpmulld (%rsi), %ymm0, %ymm1
\tvmovdqu %ymm1, (%rdi)
\taddq $32, %rsi
\taddq $32, %rdi
\tcmpq %rdx, %rsi
\tjb .Lloop
\tret
";
        let (out, n) = run(src);
        assert_eq!(n, 1, "{out}");
        assert!(out.contains("\tvmovd %eax, %xmm0\n"));
    }

    #[test]
    fn kill_via_vzeroupper_absorbs() {
        // vzeroupper zeroes every upper half: reads after it cannot observe
        // the merge, writes before it promote.
        let src = "\
g:
\tmovss (%rdi), %xmm1
\tvzeroupper
\tvaddps %ymm1, %ymm0, %ymm0
\tret
";
        let (out, n) = run(src);
        assert_eq!(n, 1, "{out}");
        assert!(out.contains("\tvmovss (%rdi), %xmm1\n"));
    }
}

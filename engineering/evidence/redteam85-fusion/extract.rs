pub fn fuse_late_compare_bool_spills(asm: &mut String) -> bool {
    let mut lines: Vec<String> = asm.lines().map(str::to_string).collect();
    let mut changed = false;

    // Inline-asm regions are user-authored bytes; this pass runs on raw text
    // (phase 9) where `LineKind::InlineAsm` pinning no longer exists, so it
    // must exclude every line inside a `#APP`..`#NO_APP` span from matching,
    // and the "slot occurs exactly 3 times" census below must ignore the
    // possibility of template text mentioning the slot (an asm mention makes
    // the count differ, which correctly suppresses the fusion).
    let mut in_asm = vec![false; lines.len()];
    let mut inside = false;
    for (idx, l) in lines.iter().enumerate() {
        let t = l.trim();
        if t == "#APP" {
            inside = true;
        } else if t == "#NO_APP" {
            inside = false;
        }
        in_asm[idx] = inside;
    }
    let asm_free = |i: usize| !in_asm[i];

    // First collapse a constant-false predecessor of a spilled-boolean join.
    // This turns the remaining compare path into a single-predecessor fallthrough.
    let active0: Vec<usize> = (0..lines.len())
        .filter(|i| !lines[*i].trim().is_empty())
        .collect();
    for p in 0..active0.len().saturating_sub(2) {
        let a = active0[p];
        let b = active0[p + 1];
        let c = active0[p + 2];
        if !asm_free(a) || !asm_free(b) || !asm_free(c) {
            continue;
        }
        if lines[a].trim() != "xorl %eax, %eax" {
            continue;
        }
        let Some(slot) = lines[b]
            .trim()
            .strip_prefix("movq %rax, ")
            .map(str::to_string)
        else {
            continue;
        };
        let Some(join) = lines[c].trim().strip_prefix("jmp ").map(str::to_string) else {
            continue;
        };
        if lines.iter().filter(|l| l.contains(&slot)).count() != 3 {
            continue;
        }
        let Some(li) = lines.iter().position(|l| l.trim() == format!("{}:", join)) else {
            continue;
        };
        let tail: Vec<usize> = ((li + 1)..lines.len())
            .filter(|i| !lines[*i].trim().is_empty())
            .take(3)
            .collect();
        if tail.len() != 3 || tail.iter().any(|&i| !asm_free(i)) {
            continue;
        }
        let load = lines[tail[0]].trim();
        let load_ok = [
            "movsbq ", "movswq ", "movslq ", "movzbq ", "movzwq ", "movq ",
        ]
        .iter()
        .any(|q| {
            load.strip_prefix(q)
                .map(|r| r == format!("{}, %rax", slot))
                .unwrap_or(false)
        });
        if !load_ok
            || (lines[tail[1]].trim() != "testq %rax, %rax"
                && lines[tail[1]].trim() != "testl %eax, %eax")
        {
            continue;
        }
        let Some(target) = lines[tail[2]]
            .trim()
            .strip_prefix("je ")
            .map(str::to_string)
        else {
            continue;
        };
        lines[a].clear();
        lines[b].clear();
        lines[c] = format!("    jmp {}", target);
        changed = true;
    }

    // Count explicit branch references; labels with none are transparent joins.
    let mut refs = std::collections::HashMap::<String, usize>::default();
    for l in &lines {
        let t = l.trim();
        if t.starts_with('j') {
            if let Some(x) = t.split_whitespace().last() {
                *refs.entry(x.to_string()).or_default() += 1;
            }
        }
    }
    let active: Vec<usize> = (0..lines.len())
        .filter(|i| {
            let t = lines[*i].trim();
            if t.is_empty() {
                return false;
            }
            if let Some(label) = t.strip_suffix(':') {
                return refs.get(label).copied().unwrap_or(0) != 0;
            }
            true
        })
        .collect();
    let mut p = 0;
    while p + 6 < active.len() {
        let ix = &active[p..p + 7];
        // User asm bytes are immutable: no fused rewrite may touch, NOP, or
        // consume a line inside a `#APP`..`#NO_APP` region.
        if ix.iter().any(|&i| !asm_free(i)) {
            p += 1;
            continue;
        }
        let t: Vec<&str> = ix.iter().map(|i| lines[*i].trim()).collect();
        if !(t[0].starts_with("cmp") || t[0].starts_with("test")) {
            p += 1;
            continue;
        }
        let Some(cc) = parse_setcc(t[1]) else {
            p += 1;
            continue;
        };
        if !(t[2].starts_with("movzbl %al, %eax") || t[2].starts_with("movzbq %al, %rax")) {
            p += 1;
            continue;
        }
        let Some(slot) = t[3].strip_prefix("movq %rax, ") else {
            p += 1;
            continue;
        };
        let load_ok = [
            "movsbq ", "movswq ", "movslq ", "movzbq ", "movzwq ", "movq ",
        ]
        .iter()
        .any(|q| {
            t[4].strip_prefix(q)
                .map(|r| r == format!("{}, %rax", slot))
                .unwrap_or(false)
        });
        if !load_ok || (t[5] != "testq %rax, %rax" && t[5] != "testl %eax, %eax") {
            p += 1;
            continue;
        }
        let (nonzero, target) = if let Some(x) = t[6].strip_prefix("jne ") {
            (true, x)
        } else if let Some(x) = t[6].strip_prefix("je ") {
            (false, x)
        } else {
            p += 1;
            continue;
        };
        if lines
            .iter()
            .any(|l| l.trim_start().starts_with("leaq ") && l.contains(slot))
        {
            p += 1;
            continue;
        }
        let fused = (if nonzero { cc } else { invert_cc(cc) }).to_string();
        let target = target.to_string();
        for q in 1..=5 {
            lines[ix[q]].clear();
        }
        lines[ix[6]] = format!("    j{} {}", fused, target);
        changed = true;
        p += 7;
    }
    if changed {
        *asm = lines.join("\n") + "\n";
    }
    changed
}
pub fn parse_setcc(s: &str) -> Option<&str> {
    if !s.starts_with("set") {
        return None;
    }
    let rest = &s[3..];
    let space_idx = rest.find(' ')?;
    let cc = &rest[..space_idx];
    match cc {
        "e" | "ne" | "l" | "le" | "g" | "ge" | "b" | "be" | "a" | "ae" | "c" | "nc" | "s"
        | "ns" | "o" | "no" | "p" | "np" | "z" | "nz" => Some(cc),
        _ => None,
    }
}

/// Invert a condition code (e.g., "e" -> "ne", "l" -> "ge")
pub fn invert_cc(cc: &str) -> &str {
    match cc {
        "e" | "z" => "ne",
        "ne" | "nz" => "e",
        "l" => "ge",
        "ge" => "l",
        "le" => "g",
        "g" => "le",
        "b" | "c" => "ae",
        "ae" | "nc" => "b",
        "be" => "a",
        "a" => "be",
        "s" => "ns",
        "ns" => "s",
        "o" => "no",
        "no" => "o",
        "p" => "np",
        "np" => "p",
        _ => cc,
    }
}


fn main() {let p=std::env::args().nth(1).unwrap();let mut s=std::fs::read_to_string(p).unwrap();fuse_late_compare_bool_spills(&mut s);print!("{}",s);}

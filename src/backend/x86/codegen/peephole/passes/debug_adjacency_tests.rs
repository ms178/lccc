//! Metamorphic instruction-adjacency coverage: debug attribution stays present
//! without disabling a fold; semantic directives and pinned code stay opaque.
use super::super::types::*;
use super::*;

type Pass = fn(&mut LineStore, &mut [LineInfo]) -> bool;

fn apply(pass: Pass, body: &str) -> (bool, String) {
    let mut store = LineStore::new(format!("f:\n.cfi_startproc\n{body}\n.cfi_endproc\n"));
    let mut infos: Vec<_> = (0..store.len())
        .map(|i| classify_line(store.get(i)))
        .collect();
    pin_inline_asm_regions(&store, &mut infos);
    let changed = pass(&mut store, &mut infos);
    (changed, store.build_result(|i| infos[i].is_nop()))
}

fn code(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !is_debug_marker_directive(s))
        .collect()
}

#[test]
fn debug_attribution_does_not_break_instruction_adjacency() {
    let cases: &[(Pass, &str)] = &[
        (
            relay_and_lea::retarget_producer_into_copy,
            "movzbl (%rdi), %eax\nmovl %eax, %edx\nshll $16, %edx\nmovl $7, %eax\nret",
        ),
        (
            memory_fold::fold_fp_register_loads,
            "movsd (%rdi), %xmm2\nvsubsd %xmm2, %xmm0, %xmm0\nret",
        ),
        (
            vector_copy::reassociate_fma_accumulator,
            "vmovsd %xmm2, %xmm2, %xmm5\nvfmadd231sd %xmm1, %xmm0, %xmm5\nmovsd %xmm5, %xmm0\nret",
        ),
        (
            local_patterns::combined_local_pass,
            "orl %ecx, %eax\nmovl %eax, %eax\nret",
        ),
        (
            relay_and_lea::fold_lea_into_load,
            "leaq (%rdi,%rsi), %rcx\nmovl (%rcx), %eax\nret",
        ),
    ];
    for (pass, body) in cases {
        let (changed, plain) = apply(*pass, body);
        assert!(changed, "missing positive control: {body}");
        for marker in [".loc 1 7 3", ".file 2 \"x.c\""] {
            let annotated = body
                .lines()
                .collect::<Vec<_>>()
                .join(&format!("\n{marker}\n"));
            let (changed, debug) = apply(*pass, &annotated);
            assert!(changed, "{annotated}");
            assert_eq!(code(&plain), code(&debug), "{annotated}");
            assert!(debug.contains(marker), "attribution was discarded");
        }
    }
}

#[test]
fn debug_adjacency_never_crosses_semantic_or_pinned_lines() {
    for marker in [
        ".cfi_def_cfa_offset 16",
        ".p2align 4,0xcc",
        ".byte 0x90",
        ".section .data",
        ".long .LBB1",
        ".location 1",
    ] {
        let body = format!("movsd (%rdi), %xmm2\n{marker}\nvsubsd %xmm2, %xmm0, %xmm0\nret");
        assert!(
            !apply(memory_fold::fold_fp_register_loads, &body).0,
            "{marker}"
        );
    }
    let body = "#APP\nmovsd (%rdi), %xmm2\n.loc 1 7 3\nvsubsd %xmm2, %xmm0, %xmm0\n#NO_APP\nret";
    assert!(!apply(memory_fold::fold_fp_register_loads, body).0);
    for marker in [".loc 1 7 3", ".file 2 \"x.c\""] {
        let store = LineStore::new(marker.to_owned());
        let mut info = classify_line(store.get(0));
        assert!(debug_transparent(&store, info, 0));
        info.pinned = true;
        assert!(!debug_transparent(&store, info, 0));
    }
}

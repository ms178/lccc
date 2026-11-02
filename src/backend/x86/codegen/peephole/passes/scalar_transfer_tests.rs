//! Execute architectural scalar-load semantics, including all upper YMM bits.
use super::*;

#[test]
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn scalar_load_forwarding_architectural_execution() {
    use crate::common::temp_files::TempFile;
    use std::process::Command;
    assert!(
        std::arch::is_x86_feature_detected!("avx"),
        "AVX fixture requires AVX"
    );
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/regression/fixtures/scalar-load-forwarding");
    let original = std::fs::read_to_string(fixtures.join("witness.s")).unwrap();
    let mut store = LineStore::new(original.clone());
    let mut infos: Vec<_> = (0..store.len())
        .map(|i| classify_line(store.get(i)))
        .collect();
    assert!(store_forwarding::global_store_forwarding(
        &mut store, &mut infos
    ));
    let forwarded = (0..store.len())
        .filter(|&i| !infos[i].is_nop())
        .map(|i| store.get(i))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let optimized = peephole_optimize(original.clone());
    assert!(
        optimized.contains("movq %xmm6, %xmm7"),
        "64-bit forwarding must remain active"
    );
    assert!(
        optimized.contains("vinsertps $14"),
        "32-bit VEX forwarding must remain active"
    );
    let asm = TempFile::try_new("scalar-transfer", "witness", "s")
        .expect("reserve a private witness path");
    let exe = TempFile::try_new("scalar-transfer", "witness", "exe")
        .expect("reserve a private witness path");
    for (stage, code) in [
        ("original", original),
        ("forwarded", forwarded),
        ("pipeline", optimized),
    ] {
        std::fs::write(asm.path(), &code).unwrap();
        let build = Command::new("gcc")
            .args(["-O2", "-mavx", "-Werror", "-o"])
            .arg(exe.path())
            .arg(fixtures.join("driver.c"))
            .arg(asm.path())
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{stage}: {}\n{code}",
            String::from_utf8_lossy(&build.stderr)
        );
        let result = Command::new(exe.path()).output().unwrap();
        assert!(
            result.status.success(),
            "{stage}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert_eq!(
            result.stdout,
            b"scalar transfers: 6720 checks, 0 failures\n"
        );
    }
}

#[test]
fn scalar_zeroing_self_moves_are_not_gp_identities() {
    for reg in ["%xmm0", "%xmm6", "%xmm15"] {
        assert_ne!(
            classify_line(&format!("movq {reg}, {reg}")).kind,
            LineKind::SelfMove
        );
    }
    for reg in ["%rax", "%r8", "%r15", "%rsp"] {
        assert_eq!(
            classify_line(&format!("movq {reg}, {reg}")).kind,
            LineKind::SelfMove
        );
    }
}

#[test]
fn scalar_masked_loads_preserve_masks_and_retire_backing_registers() {
    for (op, store_op) in [("vmovsd", "movsd"), ("vmovss", "movss")] {
        for mask in ["{%k1}", "{%k1}{z}"] {
            let masked = format!("{op} 16(%rsp), %xmm7{mask}");
            let text = format!(
                "f:\n.cfi_startproc\n    {store_op} %xmm6, 16(%rsp)\n    {store_op} %xmm7, 24(%rsp)\n    {masked}\n    {op} 24(%rsp), %xmm8\n    {store_op} %xmm7, 32(%rsp)\n    {op} 32(%rsp), %xmm9\n    ret\n.cfi_endproc\n"
            );
            let mut store = LineStore::new(text);
            let mut infos: Vec<_> = (0..store.len())
                .map(|i| classify_line(store.get(i)))
                .collect();
            assert!(!matches!(
                classify_line(&masked).kind,
                LineKind::LoadXmmRbp { .. }
            ));
            assert!(store_forwarding::global_store_forwarding(
                &mut store, &mut infos
            ));
            let out = (0..store.len())
                .filter(|&i| !infos[i].is_nop())
                .map(|i| store.get(i))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(out.contains(&masked), "masked load must survive: {out}");
            assert!(
                out.contains(&format!("{op} 24(%rsp), %xmm8")),
                "masked write invalidates the old xmm7 backing value: {out}"
            );
            assert!(
                !out.contains(&format!("{op} 32(%rsp), %xmm9")),
                "fresh post-mask backing facts must still forward: {out}"
            );
        }
    }
}

#[test]
#[ignore = "requires AVX512F; execute explicitly on a capable host"]
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn scalar_masked_loads_native_avx512() {
    use crate::common::temp_files::TempFile;
    use std::process::Command;
    assert!(std::arch::is_x86_feature_detected!("avx512f"));
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/regression/fixtures/scalar-masked-forwarding");
    let original = std::fs::read_to_string(fixtures.join("witness.s")).unwrap();
    let mut store = LineStore::new(original.clone());
    let mut infos: Vec<_> = (0..store.len())
        .map(|i| classify_line(store.get(i)))
        .collect();
    store_forwarding::global_store_forwarding(&mut store, &mut infos);
    let forwarded = (0..store.len())
        .filter(|&i| !infos[i].is_nop())
        .map(|i| store.get(i))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let pipeline = peephole_optimize(original.clone());
    let asm =
        TempFile::try_new("scalar-masked", "witness", "s").expect("reserve a private witness path");
    let exe = TempFile::try_new("scalar-masked", "witness", "exe")
        .expect("reserve a private witness path");
    for (stage, code) in [
        ("original", original),
        ("forwarded", forwarded),
        ("pipeline", pipeline),
    ] {
        std::fs::write(asm.path(), &code).unwrap();
        let built = Command::new("gcc")
            .args(["-O2", "-mavx512f", "-Werror", "-o"])
            .arg(exe.path())
            .arg(fixtures.join("driver.c"))
            .arg(asm.path())
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{stage}: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        let output = Command::new(exe.path()).output().unwrap();
        assert!(
            output.status.success(),
            "{stage}: {}\n{code}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            output.stdout,
            b"masked scalar transfers: 1536 checks, 0 failures\n"
        );
    }
}

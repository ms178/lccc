# PR829 / PR830 synthesis audit — 2026-10-15

## Identities and acceptance status

- Baseline main: `7f9eb225def23bda9370d3349304b9b785554fea`, already includes PR830 (`d508ea1c97df104c6d35d35d63749c1fe9af519b`).
- Compared PR829: `7110d5af8fc2eb9cf64d7907af4b8ed0bed24acd`.
- Prior S47: `7e633eb46cc0fd40043f9cfb12c04764a8b18be2`.
- S49 rebased preservation: `4dfbf58daafdbc1b434b06600febf0325e3e5af8`.

**INTEGRATION PENDING VALIDATION.** Historical reports are not fresh acceptance. No production-ready or zero-defect claim. Only fast CI is permitted locally.

## Source reconciliation

The rebase preserves PR830's fail-closed benchmark oracle exactly. Compiler source after the rebase is byte-identical to S47; the additional PR830 corrections are in tooling. CI parity caught and removed a duplicated IVSR structural gate introduced by overlapping histories. Resource isolation, hosted error reporting and the 154-check boundary suite remain registered.

PR829 adds independently reviewable corrections absent from S47/main: real signed-wrap option propagation and range-sensitive IV widening; explicit observable-FP environment policy; transactional I32 DAG packing for ordered reciprocal reductions; AVX2 gather-mask effect barriers; implicit architectural-state corrections; and RISC-V dynamic binding/absolute-symbol relocations. These are integrated for testing, not certified on the strength of their PR report.

Retained S47 work includes checked RV64 AUIPC endpoints, UTF-8-safe suffix admission, checked byte-load spans and the signed/unsigned affine trajectory proof. The new RISC-V binding code is merged with—not substituted for—the AUIPC corrections.

PR829's second unsigned loop-bound implementation is NOT imported: S47 already owns signed and unsigned nonwrapping affine folds, including the final exit observation, backedge dominance, non-SSA refusal and shared increment anchors. Nor is PR829's blanket `-fwrapv` prohibition imposed on this proven fold; it was needed for the older unsafe implementation. Existing scheduling is retained pending comparative measurements, rather than silently introducing an additional transformation owner. The separate PR829 early-byte-load switch and x86-throughput profitability scope are retained for causal experiments.

PR829's existing FP-containing-function fences are conservative correctness provisions, not a constrained-FP IR or complete Annex-F implementation. Remaining explicit FP-option coverage must be tested. No duplicated bulk PR evidence was imported; original source and historical report remain available at the exact PR commit.

## Toolchain and experiment integrity

Fresh provisioned versions: Debian GCC/G++ `16.2.0-3`; stable Rust `1.99.0` and cargo `1.99.0`; rustfmt/Clippy installed; MSRV `1.97.0` installed separately. Swap: 6 GiB. Compiler builds use only fastbuild, Rust O1, and `-j2`.

The first attempt at sharing a Cargo target directory across control worktrees produced identical binary hashes and instant 'fresh' builds. **Those main/PR829 controls were rejected before comparison**: relative dependency paths and worktree mtimes are not an independent rebuild. Controls are being rebuilt in separate target directories; build logs, source IDs and binary hashes are required. This failure is not counted as evidence for any PR.

Checkpoint history through S46 was losslessly transcoded into an indexed XZ tar archive to stay below the workspace preservation cap. Every patch's original bytes were verified before replacing its individual gzip encoding. No historical patch content was discarded.

## Fresh observations so far

- CI command parity after integration: 230 commands, 24 invocation contracts; PASS.
- Rebased S47 still faults on the accepted `-fwrapv` address-product witness (exit 139).
- A dynamic-rounding probe returns the wrong nearest-rounded value at S47 O2, versus correct upward rounding at S47 O0 and GCC16.2/Clang23.1.3 O0/O2 with `-frounding-math`. PR829 and candidate results are pending.

Detailed empirical results, regression decisions and final acceptance will be appended only after execution.

## Newly reproduced gaps beyond PR829's tests

The isolated PR829 build has SHA-256 `04d1899aafd54311195b70940fddcd85e262942a7d6808ae1cf5fcf8f14302cc`. Its own four wrap-domain cases return the expected 28 / -1073665362 / 1 / 10. Nevertheless, the prior **char-pointer** `p[i*4]` witness still exits 139, including with IV widening disabled. Final IR preserves I32 arithmetic; the generic backend scaling proof still assumes narrower signed operations cannot wrap. This is distinct from the repaired IV widening example with int elements.

A first synthesis build (`766edc1ae444dd937541a1fe70d33d4e0a9d950f3ed3491d39a8838b9b4099b7`) repairs that witness at O0/O1/O2/O3/Os/Oz by sharing per-TU arithmetic semantics with backend selection and retaining the narrow producer before pointer extension. It does not disable SIB selection globally. Broader testing then found a separate pre-existing lowering-width defect: `p[i*4-3]` fails even at O0 because the storage annotation suppresses a required I32-to-pointer-width cast. Array subscripts and both pointer-add orientations now consult the existing actual-value-width helper; validation is pending.

PR829 also rounds `1.0 + 0x1p-53` to nearest even with active FENV_ACCESS and runtime FE_UPWARD. Pass checkpoints identify **cleanup-sccp** as the first wrong transformation: its direct constant-evaluation oracle bypasses the constant-fold pass fence. The shared binop/unary/compare/cast evaluator now refuses observable-FP evaluation, protecting SCCP and IPCP too. IP purity and constant-return inference must not erase exception-producing callees. The first synthesis build passes the simple upward-rounding probe both via pragma and the newly explicit `-frounding-math` option.

Expanded rounding tests reveal another bypass: runtime explicit constant casts are folded during lowering, before the old policy initialization point. The same per-TU policy is now installed before frontend lowering as well as at every pass entry, and runtime FP cast folding respects it. Static initializer evaluation retains its distinct translation-time contract. Full validation remains pending.

The new full FENV probe distinguishes references rather than voting them correct: Clang23.1.3 passes all four rounding modes and the discarded-result exception observation at O0/O2. GCC16.2 passes its rounding portion but drops the unused division at O2 (exit 52); that result is recorded, not used to excuse a lost observable exception.

Initial RV binding invocations mistakenly used the x86 dispatcher and are excluded. Corrected runs use each independently built unified compiler under its documented `lccc-riscv` argv[0] target selection. Likewise cross-boundary runs require sibling target names. The corrected S47 boundary run passes all 154 checks. Raw invocation mistakes are retained as harness errors, not compiler defects.

### Follow-through native validation and expanded cross-layer audit

Follow-through binary SHA256 `351b905c1d695d0c1db10c9558407a7ee4ed781c7de3217b2e84a232f3f564d7` passed both new witnesses at O0/O1/O2/O3/Os/Oz (12/12), plus the wrap witness at i686 O2. Expanded pointer-left/right/subtraction/compound-assignment coverage also passed all six native optimization levels. These are fresh executable results, not the old S47 gates.

The permanent boundary matrix now includes wrap byte addressing and a libc-independent dynamic-rounding witness; the FP matrix includes both native and i686, all six optimization levels, and pragma/flag/both modes. The first expanded cross run stopped at 151 passes: AArch64 O1 dropped the signed extension when folding a negative I32 offset into a register-offset byte load. Native FP passed O0 in all three modes, but i686 compilation exposed an invalid F64-to-Void IR cast. These are additional actual failures, not test relaxations.

Pending validation: void casts now evaluate their operand without generating a conversion; AArch64 indexed addressing tracks actual IR widths, uses SXTW/UXTW register offsets, and explicitly extends narrower indices. Its access-scale guard also now enforces the ISA's natural-width scale (the prior unit test incorrectly accepted F32 with shift 3). Unknown index types fail closed before producer suppression. No blanket optimization disable was introduced. Source, unit tests, cross execution, full fast CI and performance still require completion for this revision.

Validated follow-through (S52 source): scripted warning-denying fastbuild O1/j2 completed in 3m24s; compiler SHA256 `9e7a2758895ddc19dc576574e47acfbfb8bb0500dc7f9084c8fefc31b4d33622`. Expanded boundary matrix **228 PASS / 0 FAIL / 0 SKIP**, including all four architectures, actual-value signed indexing and freestanding dynamic rounding. FP shell matrix also completed: 18 original trap-order cases and **36 dynamic-rounding/discarded-exception cases** across x86-64/i686, six optimization levels and pragma/flag/both. Additional unit tests assert actual-width extension and fail-closed unknown widths. Full gates/performance remain pending.

### Pre-PR831 continuity checkpoint (2026-10-15)

The harness reset before the requested rebase. S53 exact history was recovered offline at 1f24cf9b585f64cd39a250ec3672d726265a00ca, tree b3013246e93368d57f2e5e4d20450670cd9da523. Source and compressed boundary/FP evidence survived. The unfinished fast-CI run, independent control binaries, and cache-only Callgrind manifests did not. None certify the restored or rebased compiler.

Prior-turn observations requiring fresh reproduction: candidate RV binding plus LLD oracle 480/0, real RV64 loader 13/0; nine packed-reciprocal arms (GCC16, Clang23 and seven candidate configurations) each matched 1260 bit patterns and passed Memcheck. A completed 45-case candidate/S47 Callgrind screen reported GM 0.997681 and AM 1.000036, with spectral_norm Ir ratio 0.644799 but TLS 1.180358 and sieve 1.118775. A partial candidate/PR829 screen showed TLS 1.053959. These transcript observations have lost their raw manifests and are NOT final acceptance evidence.

Assembly inspection localized the additional TLS cost to retaining signed narrow subtract/add producers under -fwrapv. This repairs a demonstrated miscompile; removing the semantic guard for speed is unacceptable. A potential follow-up is to reuse the existing width-aware, unique-definition integer-bounds analysis to prove selected operations nonwrapping, not add a workload-specific exemption or a duplicate range analyzer. No such optimization was implemented before the reset. Native runtime comparisons and a thorough regression investigation remain outstanding.

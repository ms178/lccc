# PR829 / PR830 / S26 comparative audit — 2026-10-15

## Scope and identities

- Prior validated S26: `47a7ce09ce0c0c080d641a0e0b8f93cc6235e934`.
- PR829: `7110d5af8fc2eb9cf64d7907af4b8ed0bed24acd` (open when fetched).
- PR830: `d508ea1c97df104c6d35d35d63749c1fe9af519b`, already merged as
  `7f9eb225def23bda9370d3349304b9b785554fea`.
- S26's 24 commits were rebased onto that main. All S26 compiler source bytes
  survived the rebase unchanged. Upstream's benchmark-gate repair and separate
  TLS entry assembly were retained; our TLS-template poisons were retained too.
- This is an empirical, bounded audit, not a proof of universal optimality or
  blanket compiler correctness. No remote push was performed.

## Reproduced comparisons

Three frozen compilers were built with stable Rust 1.99.0, fastbuild/O1, -j2,
no LTO, no debug information, GCC/bfd linking. Compiler SHA256 identities are
in `evidence/pr829-830/compiler-hashes.txt`. The S26 binary SHA matches the
previous session exactly despite the runtime reset. An 8 GiB swap file is active.
Local GCC 16.2.0-3, Clang/lld 23.1.3, binutils 2.47 and Valgrind 3.27.1 were
installed and version-checked. Compiler Explorer's GCC16.2, Clang23.1.0 and
latest ICX were queried for all 15 requested screening cases, preserving each
registry variant's macro flags. These are static-codegen comparisons, not
hardware timing. The external EPYC report is a screening input, not this VM.

### Correctness

The same 86 native configurations gave S26 **80 pass / 6 fail**, PR829 **86/0**,
and PR830 **68/18**. The signed-overflow fixtures explicitly use `-fwrapv`.
S26's failures were four narrow signed address-wrap executions and two FP
first-trap order cases. PR830 additionally reproduced the signed-affine
`2/95` and rotated `2/11` failures; the expected result is `5/97`.
Clang passed 86/86. GCC passed 85/86: its O3 first-trap result differs. GCC's
handling of `FENV_ACCESS`/trapping assumptions is not interchangeable with
Clang's; this is not evidence that agreeing with GCC alone establishes the
requested FP-environment contract.

Independent unit-test transplantation exposed **20 failures / 24 tests** in
S26: one gather-mask test and 19 implicit-effect rows. The same seven existing
S26 fixed-register/masked-copy tests failed **7/7** on each PR. Those are real
production-pass failures, not renamed benchmark counts. Test-only baselines
were built only after freezing the original compiler executables.

PR829's RISC-V changes pass 240 structural binding cases and 13 real glibc
loader/QEMU cases. S26 and PR830 each fail 230/240 structural cases. Initial
wrong-driver invocations were infrastructure errors and were discarded, not
counted as compiler defects.

The PR829 oracle was too prescriptive for protected symbols: symbolic
`R_RISCV_64` can be locally bound just like RELATIVE. The repaired oracle
compares loaded addresses under two load biases, while preserving ABS's
no-load-bias rule. Four new positive/negative controls bring its selftest to
14 tests. LLVM lld23.1 passes all 240 cases. GNU BFD2.47 passes 212/240 after
removing 34 false mismatches; 28 disagreements remain. A separate real-loader
run independently fails BFD's protected-absolute case (12/13 pass), whereas
PR829 passes 13/13. Neither oracle disagreement nor oracle majority is a proof.

### Cost and generated code

Fresh 45-case Callgrind comparisons passed executable correctness checks on
both arms, using the bounded registered corpus and common `-fwrapv` flags.
PR829/S26 whole-program simulated Ir includes spectral_norm 0.644826,
nbody 0.986709, but sieve 1.118728, tls_seg_access 1.119559,
sqlite_varint 1.021599 and strlen_bench 1.011012. Correctness fences have a real
cost; these cannot honestly be advertised as across-the-board wins. PR830/S26
includes byte_load_reassembly 1.142834, expat_xml_scan 1.010087 and nbody
1.029707. Small whole-program count differences are not timing superiority.

The initial synthesis is instruction-count-identical to PR829 on all 45
paired cases. Its native 86-case matrix is green and its Rust library tests
are **4745 pass / 0 fail / 8 ignored**. The signed control/rotation gate passes.
The unsigned gate exposed a composition issue: the new earlier unsigned fold
consumed the late fold's positive witness and did not share its kill switch.
The synthesis repairs that integration: both folds now share the thread-local
enable/debug controls. Positive tests separately exercise the early fold and
the late fallback; the existing kill-switch negative checks remain mandatory.

## Choices and architecture

Retain S26's GP fixed-operand constraints, EVEX masked-copy refusal, full
signed/unsigned/clone trajectory proofs, byte-load ParamRef/home bridge,
GCC16 multilib discovery, ARM input/ADR/x17 and i386 DT_RELENT repairs.
Retain PR830's mandatory oracle failures and its improved TLS entry fixture.
Adopt independently tested PR829 wrapping-policy plumbing, FENV barriers,
per-instruction implicit effects and gather barriers, RISC-V binding repairs,
early bounded unsigned guards, and strict packed integer denominator DAGs.
The packed reduction preserves scalar accumulation order and has a separate
FENV fence; it is not permission to reassociate FP arithmetic.

The initial strict packed reciprocal test passes 1260 bitwise cases in each
of nine compiler/disable/ISA arms, all with clean Memcheck. Native trap tests
also cover empty/negative loops and option precedence. A missing 32-bit loader
debug image was correctly reported as infrastructure failure, not a pass;
the exact loader debug image was restored and all 24 wrapping-domain
ABI/oracle Memcheck cases then passed. The readonly-reduction helper also
needed a PATH-resolution repair: a bare `gcc-16` must be resolved with
`shutil.which`, not treated as a file under the checkout.

The benchmark-output gate needs more than PR830's empty-filter fix. Four
hermetic controls still fail on PR830: matching crashes/nonzero exits,
trailing-newline differences and embedded-NUL differences. The replacement
uses the benchmark registry (including shared-source variants), exact stdout
bytes, mandatory successful exits and common wrapping flags. TCE alone uses
4096 recursive calls at O0/O1; the ordinary benchmark default remains ten
million and its default output spelling is unchanged. All 16 hermetic gate
controls pass, including empty/duplicate registries, empty include-provider
output and subprocess timeouts. Selection covers all 45 registered shapes
plus 15 legacy probes (60 cases / 58 distinct sources); it does not silently
replace the legacy corpus with the registry. Six compiler arms emitted
byte-identical default O2 TCE assembly with/without the explicit original
bound; all 12 bounded O0/O1 executions agreed exactly. Full/slow benchmark CI was not run locally.

## Rejected range-recovery experiments

A typed controlling-header range proof was implemented and tested, including
signed sums/products and the first failing IV, not just body iterations.
The broad version passed 4748 Rust tests, all 86 native configurations and
the signed/unsigned affine gates. Nevertheless, its direct 45-case Callgrind
comparison against the initial synthesis regressed nbody to **1.0210022185**,
versus matmul **0.9986108923** and SHA256 **0.9932925540**; 42 cases tied.
The geometric mean was **1.0002814751**. Extra pointer-width recurrence live
ranges are not free merely because the arithmetic proof is sound.

A second version restricted recovery to genuinely narrow signed products.
It tied the initial synthesis on all 45 instruction counts: no measured
benefit justified the extra proof machinery. **Neither experiment ships.**
Their patches, compiler hashes and measurements are preserved as rejected
experiments, not presented as accepted optimizations. In the raw evidence,
`proof` names the broad experiment and the historical scratch label `final`
names the rejected narrow experiment; `accepted` names the S29 pre-FAST candidate, and `selected` names the
compiler after the FAST-discovered byte-scheduling integration repair. The broad compiler-work sweep was deliberately stopped and retains
`complete:false`; its partial three spectral rounds are not a completed
four-source qualification.

The S29 compiler was rebuilt after removing both prototypes. Its native
matrix passes **86/86**, and a fresh direct Callgrind sweep ties the initial
synthesis on **all 45 cases**, with no execution failures.

## FAST-discovered byte-scheduling composition regression

S29's first FAST attempt correctly failed the existing byte-accessor gate:
the PR829 earlier x86-64 placement bypassed `postinline`'s historical suppression
of the extra scheduling opportunity. It also removed S26's i686 placement.
Independent assembly probes reproduced two remaining `body_le64` calls on
i686 for both PR829 and S29, versus zero in S26 (all three had zero on x86-64).

The repair does not weaken the gate: the x86-64 extra scheduling opportunity
now honors the existing `postinline` diagnostic contract, and i686 retains
S26's bounded straight-line/private-helper placement without pre-inline DCE.
The regression adds explicit i686 O2/O3 shape checks and tests both disable
controls on both ABIs, in addition to its 20 native executions. The failed
FAST receipt remains evidence; only a fresh green run can certify delivery.
The repaired compiler passes the expanded byte gate and **86/86 native
configurations**, and directly ties PR829 on **all 45 x86-64 Callgrind cases**.
The S30 pre-final-FAST binary SHA256 is
`6914628e49cf7e41c0709329d0590c90d6cf812b857cfa97d72410f7eac6bab2`.

## Architecture and bounded acceptance rationale

- `src/passes/iv_strength_reduce.rs`: keep wrapping-domain refusals instead of
  replacing narrow modular arithmetic with pointer-width recurrence. Preserve
  the validated S26 x86 scaled-address exception and Copy/cast safeguards.
- `src/passes/loop_rotate.rs`, `loop_bound.rs`, `mod.rs`: preserve the S26
  controlling-header/cloned-trajectory proof and share enable/debug policy
  across early unsigned and late affine folds. Per-pass disable remains
  separate from the global affine kill switch.
- `src/passes/vectorize.rs`: adopt tested strict packed denominator work and
  FENV legality, but retain tail, mask and memory-dependence conservatism.
- x86 peepholes: retain S26 fixed-register and masked-copy counterexamples
  alongside PR829 implicit-effect and upper-lane barriers. A whole-PR winner
  is not a sound substitute for composition tests.
- RISC-V linker: select address/preemption semantics backed by independent
  ELF algebra and real QEMU loader tests. Each 480-row paired receipt is
  **240 LCCC records + 240 oracle records**, not 480 distinct fixtures; two
  load biases are checked inside the semantic model. The candidate also
  passes all 13 real-loader cases. BFD's 28 model disagreements are not all
  independently demonstrated runtime bugs.
- Infrastructure: extend existing registry/oracle/profiling tools instead of
  keeping overlapping unused codegen-cost and evidence-repacking scripts.
  Recovery accepts authenticated gzip or xz source tarballs under identical
  member safety checks; 31 recovery tests include offline xz exact-history
  reconstruction and authenticated solid-archive bundles (including corrupt,
  duplicate, escaping, missing and symlink-member rejection). Compression must not destroy named snapshots or history.

PR829/S26's 45-case geometric Ir ratio is **0.9967351**, arithmetic **0.9989033**;
PR830/S26's are **1.0045901** and **1.0048059**. These small aggregate changes
hide substantial per-case tradeoffs. In particular the validated synthesis
retains roughly 35.5% fewer simulated instructions for spectral_norm, but
sieve and TLS remain about 11.9% worse than S26. No claim of universal compiler
superiority, EPYC timing reproduction, or expert endorsement is made.

## FENV composition failure discovered by FAST

The next FAST run exposed a real PR829 regression beyond the initial 86
witnesses: `fp_recurrence/cases.c` at O2 changes a dynamic-rounding result from
`3fdff7c429b868b9` to `3fdff7c429b868b8` (exception flags remain `20`).
S26 and PR830 pass; PR829 and the S30 synthesis fail. An independent disable
matrix isolates backedge PRE (`bepre`), not loop-square reuse, vectorization,
peepholes or register allocation. It carries the previous square across
`fesetround`, although equal operands no longer imply equal FP expressions.
The new FENV barriers changed the surrounding IR enough to expose this latent
legality gap.

Backedge PRE now uses the same observable-FENV fence, including FP casts;
integer-only functions remain eligible. The existing bitwise/dynamic-rounding
fixture stays mandatory. Contraction shape positives now use a separate
no-FENV source; the original FENV-enabled source explicitly asserts refusal
even with `-ffp-contract=fast`. This is a semantic correction, not permission
to weaken the exception/rounding checks. The final receipt supersedes all
S29/S30 preliminary candidate hashes and failed FAST attempts.
The portable harness now includes eight dynamic-rounding configurations:
**94/94 pass** for the repaired compiler and Clang, **88/94** for S26,
**90/94** for PR829, **76/94** for PR830, and **89/94** for GCC. GCC's four
additional failures reproduce the same dynamic-rounding difference even with
`-frounding-math`; they are FP-contract compatibility gaps, not a reason to
accept the transformation by oracle voting. The unoptimized volatile control
and Clang agree with the expected result.

The repaired compiler also passes the complete FP recurrence/contraction/
x87/jump-thread gate and directly ties PR829 on all 45 x86-64 Callgrind cases.
Its SHA256 is
`cd07f94ca5f42f0d847e5cc18cb22fd24b686d17315e9ec13674df3b67b9c766`.

The restored i686 byte placement measured **82,469,136 / 103,229,131 =
0.7988940254** whole-program Callgrind Ir versus PR829; about 20.1% fewer
simulated instructions, not a native timing claim. Final vendor/static and
compiler-work receipts are external to the immutable tested tree.

## Tools, reproducibility and remaining debt

- All retained compiler artifacts use fastbuild, Rust O1, fixed -j2, with
  debug information and incremental state disabled on the constrained VM.
  An accidental invocation of the release helper was stopped and discarded;
  no release artifact was used for validation or publication.
- Pinned x86-64 and AArch64 GAS/objdump pairs report **2.47.20260726**. The
  stalled FTP fetch was retried through a mirror, then the unchanged pinned
  provisioning scripts built/verified the tools.
- Official private mold **2.42.1** passes native m64/m32 TLS smoke tests. It is
  not a source build or full qualification. The exact source CMake selector
  is `-DMOLD_TARGETS='X86_64;I386'`.
- The local machine is a two-vCPU Intel Xeon KVM VM, not the reported EPYC7763.
  Valgrind 3.27.1 Callgrind Ir is simulated work, not cycles or elapsed speed.
  No PMU measurement was performed. Historical Valgrind 3.24 absolute counts
  must not be chained with these counters.
- The three hosted vendor results cover all requested worst15 variants;
  whole-translation-unit static counts are not runtime rankings or proof
  that every stack reference is a spill. Original EPYC flags and PMU data
  were unavailable. Native FENV semantics, not oracle voting, decide traps.
- Remaining risks: conservative wrapping-policy costs; monolithic optimizer
  modules; unproven profitability outside this bounded corpus; GNU BFD model
  differences; and hosted full/slow CI, which is intentionally **not run
  locally**. A nonallocated RISC-V relocation probe had no independent
  defined-C oracle and was not promoted into a speculative compiler change.

## Immutable final acceptance

The final gate receipt is external to the tested Git tree to avoid changing
that tree when recording its own acceptance. See
`/home/user/artifacts/pr829-830/FINAL-ACCEPTANCE.json` and its accompanying report for
HEAD/tree/base, final compiler hash, latest-main fetch, `ci_local.sh --fast`,
rustfmt, strict clippy, final native/Memcheck/profile results, patch/archive
hashes and restore checks. An absent or unsuccessful receipt is not a pass.
No full/slow CI or remote push is part of this local delivery.

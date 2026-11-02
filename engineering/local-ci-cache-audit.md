# Cache-backed local CI follow-up

The full local mirror exposed failures outside the focused scalar-count checks.
They are addressed without suppressing compiler gates or converting failures to
skips. Toolchain archives come from the repository's cache tags and are checked
against their manifests; native and assertion-enabled builds use `fastbuild`.

## Registry and diagnostic contracts

The live benchmark registry has 49 entries. Comparison/publication fixtures now
assert the size of that complete registry instead of a stale 46-entry snapshot.
The named-candidate test retains an explicit 49-entry latch and requires the
scalar-facts and word-count workloads. Missing, duplicate, unknown, failed,
unstable, short, and inconsistent raw observations remain negative controls.
The provisioning test checks the actual bounded annotation title while retaining
its exit-status, percent/newline escaping, and no-qemu-after-GCC-failure checks.

## Regression comparisons

Three fixtures previously executed only the LCCC arm because their GCC build
failed. `tests/regression/bmi2_bzhi.flags` explicitly enables BMI2 on both arms;
`tests/regression/vex_move_false_dep.flags` supplies libm; the apply/return fixture
casts the callee to the generic function-pointer type required by GCC's apply
API rather than suppressing an incompatible-pointer diagnostic. These preserve
the fixture algorithms and make their oracle comparisons meaningful.

## Documentation scope

The documentation gate excludes only root `.cache` and `work` trees from project
document enumeration and basename lookup. These contain third-party SDK docs and
ephemeral experiment reports. Nested project directories with those names are
still scanned. Backtick scratch-output citations are provenance, but explicit
Markdown links to missing scratch outputs still fail. Tests also prove that a
vendor basename cannot satisfy a broken project link and that new project docs
and source-path citations remain checked.

## Audit ratchet accounting

The previous baseline predates two changes already in main
`91bbcda81fbcb9750298f9869fb5b990d9d819f5`:

- `src/backend/x86/codegen/atomics.rs` adds an internal unreachable test-and-set match arm in
  the wide atomic-RMW emitter: test-and-set is byte-wide, not a wide-RMW arm.
  This accounts for main's 835 panic-class sites versus the stale 834 baseline;
  hot parser/lowering/driver sites stay at 105.
- `forward_dominated_copies` in `src/passes/sret_copy_elision.rs` crosses the
  300-line threshold while retaining its per-copy ownership and refusal checks.
  Main and this follow-up have 230 functions above that threshold, not 229.
- The same main pass adds `CCC_NO_SRET_COPY_ELISION`: 512 knob literals versus
  511. Its presence-based behavior is now documented in
  `docs/optimization-passes.md`; undocumented knobs stay at 279 rather than
  raising that allowance.

The count matcher itself no longer adds a panic-class site. Its nonzero-opcode
refinement returns an optional result and refuses unsupported unary operators.
A unit test covers all four accepted count opcodes and non-count refusals. Thus
its contribution does not require enlarging main's panic budget. Other ratchets
(unsafe, ignored tests, hot panics, functions above 500/1000 lines) are unchanged.

## Evidence boundaries

The recorded scalar-count timing experiment belongs to its recorded compiler
and source-tree identities; this follow-up does not relabel it as a new run.
Local gate logs, exact tool/runtime versions, source identities, and any binary
comparison supplement are recorded separately under the ignored `work/local-ci`
workspace. Sanitizer SDK link interfaces use the host Debian runtimes, with
mutation, undefined-behavior, and leak controls executed rather than disabled.
A local result is not a claim that unavailable authenticated vendor jobs or a
different Rust release have been reproduced.

## Explicit code models in custom linker fixtures

Nine custom fixtures bypass the common `Case` constructor. Pin their C main
programs to PIE-compatible code when reused in PIE links, rather than relying
on Ubuntu's default-PIE GCC configuration. This also avoids an executable PLT
address changing the IFUNC address-equality probe. Pin the local-typeinfo C++
fixture to PIC so its dynamic absolute-vtable-relocation assertion tests the
intended input model, not a toolchain-default copy relocation. Handwritten
assembly, structural relocation checks, runtime expectations, and intentionally
non-PIC rejection cases are unchanged. A command-contract test exercises all
nine construction paths with a failing compiler stub, proving that each pins
its own model without masking compiler failures.

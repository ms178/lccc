# Follow-up 2026-09-20 — CI review audit and verification hardening

## Critical assessment

The review's six verification findings were independently checked rather than
accepted on inspection.

* **F1 confirmed.** The old regex `^\.L(c|t)z_nz_` cannot match either real
  spelling (`.Lclz_nz_N` / `.Lctz_nz_N`). Constructed positive input now counts
  two labels. A compiled unguarded defined-zero CLZ/CTZ pair counts two, while
  the CFG-proven nonzero regression counts zero.
* **F2 confirmed.** A tree carrying only `.lccc-tools-stamp` had no basis for
  deciding whether CC changed, so preserving objects on migration was unsafe.
  Migration now performs exactly one authoritative Kbuild clean and retires the
  legacy stamp atomically with publication of separate hashes.
* **F3 confirmed and broader than reported.** The ELF32 asm differential was
  absent from hosted CI. A complete command-path audit found sixteen additional
  standalone fast local contracts absent from all hosted workflows. They are
  now grouped in the test job, and `check_ci_gate_parity.py` makes future drift
  a failing invariant across all workflow files (the expensive codegen gate is
  correctly found in `bench.yml`).
* **F4 accepted as defensive hardening.** An empty interval denotes an
  unreachable contradiction, where specializing is not a semantic
  miscompile, but vacuous truth is the wrong API contract for
  `proves_nonzero`. Empty facts are now rejected and directly unit-tested.
* **F5 confirmed.** The simultaneous CC+LD case did not independently pin the
  compiler-only `if/elif` path. Migration, linker-only, compiler-only,
  simultaneous-change, unchanged, and stamp-retirement cases are now separate.
* **F6 confirmed.** Validation occurred in the caller directory but hashing
  after `cd K`; relative paths therefore passed one stage and failed the next.
  Both tools are canonicalized and hashed before changing directories, with a
  relative-argument regression.

The review's broader statement that the original nonzero frontend lowering was
semantically complete was **not accepted**. Although ISO C leaves direct
CLZ/CTZ zero input undefined, this compiler has an intentional, regression-
pinned deterministic extension (`Clz(0)/Ctz(0) == width`). Hosted CI exposed
three failures when direct builtins were changed to NonZero. The proper design
keeps direct builtins defined and lets CVP specialize only under a dominating
nonzero proof. CVP now follows same-width/widening integer casts (which preserve
the zero partition) and rejects truncations. Operand lowering uses the proper
unsigned suffix width, eliminating a redundant RISC-V/AArch64 sign extension.

## Validation evidence

* Constructed guard-regex positive control: 2/2 labels matched.
* Compiled defined-zero negative-path control: 2 guard labels.
* Compiled CFG-proven nonzero corpus: 0 guard labels.
* Previously red corpus cases pass differentially:
  `bitop_nonneg_zext`, `bitops_builtins`, `guarded_clz_ctz_ternary`.
* Full available regression sweep: 767 pass; only three i686 environment
  failures from missing local multilib headers/runtime (hosted CI installs
  these packages); none of the reported semantic failures remain.
* Kernel tool identity hermetic suite passes all independent invalidation cases.
* ELF32 asm differential: 21 passed, 0 failed.
* CI/local standalone gate parity is enforced dynamically; after rebasing onto
  upstream PR #563 it immediately caught three newly added local contracts,
  which are now also represented in hosted CI.

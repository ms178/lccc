# Follow-up: PR #555 red-team audit — finish line and hardening

Date: 2026-09-19 (session 55)
Base: PR #555 `cfe6a4bc` ("Harden x86 codegen and boot harness regressions")
on upstream `11e3a1cd` (merge of #554). Audit commit: see `git log` for
`pr555-audit`.

## What this session did

PR #555 was red on CI (the Clippy job). Root cause: the job runs
`cargo fmt --all -- --check` FIRST; the PR's tree had five formatting
divergences (encoder/mod.rs test literal layout, a doubled blank line in
passes/mod.rs, a brace placement in lvalue.rs), so the job died at the fmt
step and clippy never ran. Local clippy on the identical stable toolchain
(rustc 1.98.1) is green on lib/bins/tests with `-D warnings` — after
formatting, the CI job is expected green end-to-end.

The full PR diff (16 files) was then audited hunk-by-hunk. Verdicts and
the additional defects found and fixed:

### PR hunks verified sound (no change needed)

* **i686 `.code16` unsuffixed stack ops** — the guarded match arms route
  `push`/`pop` to the 16-bit encoders exactly when `code16 && !code16gcc`;
  the `fixup_code16_prefixes` inversion then produces GAS-identical bytes
  in all three modes (verified against local binutils: `.code16 push $0`
  = `6a 00`, `.code16gcc push $0` = `66 6a 00`, `.code32` = `6a 00`).
* **x86 parser hex-constant vs symbol-difference** — `is_label_like` now
  rejects anything `parse_integer_expr` accepts. This is correct beyond
  the kernel driver (`init_top_pgt - 0xffffffff80000000` →
  SymbolPlusOffset with addend `0x80000000`): bare integers can never be
  GAS numeric local labels (those are always `Nb`/`Nf` spellings, which
  still fail integer parsing), so `sym - <integer>` expressions stop
  manufacturing phantom SymbolDiff symbols. All other is_label_like call
  sites (displacement and data-value paths) inherit the same correction.
* **emit.rs typed-call home-def** — sound. `try_lower_call_typed` builds
  `CallRetMove { dst: Reg(home) }` exactly when `reg_assignments` holds
  the dest (the Slot form never has a register assignment, so the outer
  `continue` already skips it), and `CallTyped` is buffered and flushed as
  its own run — nothing can follow the ret-move inside the window, so
  "a call result re-establishes its home unconditionally" is exact.
  Follow-up note: this fix has no unit test; emit.rs has no test module
  (corpus-level coverage is the established pattern there — the
  differential oracle + regression suite carry it).
* **lvalue/structs pointer-to-array subscripts** — the type-driven
  `result_is_array` predicate (Array/Pointer whose element is an Array)
  matches C semantics for `T(*)[N]` subscripts and works whether or not
  the base type is decayed; the structs.rs arm ordering is safe because
  `struct_layout_from_ctype` declines non-struct elements.
* **aggregate_copy_forward partial spans** — the `spans_parallel` guard
  converts a metadata-mismatch ICE into a no-op sync; computed before the
  instruction remove in both sites. Correct.
* **vectorize.rs const-trip ISA gate** — correct and necessary (see the
  vec_arx finding below for the proof that the failure mode is real).

### Defects found by the audit and fixed in this session

1. **`rcx_is_live_at` kill detection was unsound** (the PR's own new
   code). The hand-rolled mnemonic-prefix list treated `cmovcc` into
   `%rcx` as a pure write — but a conditional move PRESERVES the old
   destination when the condition is false, i.e. reads it — and treated
   `setcc %cl` / `movw %ax, %cx` as kills — but sub-width writes leave
   the surviving bits of the copied 64-bit value observable. Three new
   unit tests fail on the PR's version and pass on the fix (reproduced
   both ways); a positive control keeps genuine full kills
   (`movq %rbx, %rcx`) eliminating. The kill test now composes the
   shared acceptance-grade predicates: `writes_family_full` (sub-width
   excluded) ∧ `implicit_read_refs == 0` (`rep` counts, `cpuid`
   subleaf) ∧ `!is_read_modify_write` (cmov) ∧ no source-side family
   mention (memory bases).
2. **`vec_arx::vec_arx_function` had no ISA gate** — the same bug class
   the PR fixed for `vectorize_const_trip_map_loops`. It runs in Phase
   2b BEFORE the main vectorizer's gate and emits XMM-shaped Vec*
   intrinsics. Proven live:
   `tests/benchmark/programs/chacha20_block.c` (the array spelling) with
   the kernel's `-mno-sse` set failed pre-fix with
   `floating-point operation requires SSE ... movdqu (%rax), %xmm2`;
   with the gate it compiles to zero SIMD refs at -O2 and -O3. The ISA
   gate script now pins both spellings (array + phi) under the kernel
   flag set.
3. **`encode_push` rejected segment registers** — `push %ds` unsuffixed
   hard-errored ("bad register") in `.code32`/`.code16gcc` where GAS
   emits `1e` / `66 1e` (binutils 2.44 adds the inert `0x66` in gcc
   mode). Added the segment arms mirroring `encode_pop`/`encode_push16`;
   the `sized_op` bookkeeping already set at the function top yields
   byte-exact GAS parity in all three modes (unit-tested against the
   verified binutils dump, including `pushl %ds` = `66 1e` in `.code16`
   and the two-byte `fs`/`gs` forms).
4. **The ISA gate script was wired into NEITHER CI mirror** — the PR
   hardened `check_vectorize_isa_gate.sh` (kernel-FMA diagnostic
   contract) but ci.yml/ci_local.sh never ran it, so the very contract
   it pins (including the PR's own vectorize.rs fix) was unenforced.
   Wired into both; ci_local now runs 45 gates green.

### Formatting (the CI blocker)

`cargo fmt --all` applied to the five divergences; `--check` is green.
Pure whitespace, no semantic change (diff reviewed line-by-line).

## Verification (final tree)

* `cargo fmt --all -- --check`: green.
* clippy lib/bins/tests, `--profile fastbuild --locked -D warnings`: green.
* `cargo test`: 2890 passed, 0 failed (2861 at the PR base + the PR's and
  this session's tests).
* Full regression suite: PASS=720 FAIL=3 SKIP=17, AB-diff 0 (the 3 =
  the pre-existing environmental i686 multilib-header failures; no root).
* `ci_local.sh --fast`: 45 passed, 0 failed, 3 skipped (new gate
  `vectorize-isa-gate` included).
* Benchmark output oracle: 204/204 PASS.
* bash -n on all changed scripts; ci.yml/bench.yml YAML-validated.
* GAS byte-parity evidence for the encoder changes captured against the
  local binutils (code16 / code16gcc / code32 dumps in the session log).

## Follow-ups

1. **emit.rs typed-call home-def unit coverage** — needs a MachInst
   window harness (reg_assignments + prior home_clobber + CallTyped in
   one window); until then the differential oracle is the guard.
2. **`.code16gcc` segment pop parity** — `encode_pop`'s unconditional
   `sized_op = true` coincidentally matches modern binutils (`66 07`)
   in gcc mode; if byte-parity with older GAS is ever wanted, split the
   flag per-arm (functionally irrelevant: the prefix is architecturally
   ignored on segment push/pop).
3. **`xorl %ecx, %ecx` as a kill** — the zeroing idiom is a genuine full
   redefinition but `is_read_modify_write`'s conservative default keeps
   the address copy; a same-register-source exception in the kill test
   would unlock it if it ever shows up in profiles.
4. **Boot harness gates** remain SKIP in this sandbox (no kernel tree);
   the boot-offset stubs and QEMU firmware-union logic are bash -n /
   logic-reviewed only. First run on a kernel-equipped host should
   confirm `ensure_boot_offset_stubs` restores deterministic setup
   offsets after a full Kbuild.

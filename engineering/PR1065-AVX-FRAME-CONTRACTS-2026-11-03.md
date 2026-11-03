# PR1065 raw AVX FP frame contracts — 2026-11-03

## Scope / non-completion boundary

Fixes the measured kernel NAP frame regression after integrating main e133.
This is **not** a claim that all thirteen warning/options families, all kernel
compilation paths, full link/objtool/QEMU boot or hosted CI are complete.
Unsupported-option warnings still occur with the actual package flags. No
warning threshold, validator, security policy or target compiler was relaxed.

## Root cause and engineering choices

27 explicit AVX FP producers lacked the SIMD home/result-width and/or exact
pointer-operand metadata already provided for add/multiply. Private min/max
and compare results therefore escaped temporary promotion and conservative
alignment relaxation, retaining numerous permanent 32-byte-aligned scratch
objects. Record their actual 32-byte widths and audit each argument position:
unary/immediate forms have one vector operand; binary/compare/shuffle/blend
forms have two; immediates are never pointer-backed vectors. Reuse the existing
promotion/alias/escape proofs rather than raising frame limits, weakening
alignment globally or adding a NAP-specific exception.

Also fix explicit 256-bit MIN/MAX dispatch's commutative flag. Intel MIN/MAX
select source two for NaNs and equal signed zeros: left-memory folding cannot
swap sources. A forced backend unit exercises both fold sides for all four
PS/PD min/max operations, rejecting the reversed left fold while retaining
the valid right-memory fold. Runtime fixtures compare every relevant lane's
bits, including signed zeros, distinct quiet-NaN payloads and infinities.

## Actual Linux evidence (LCCC only, never GCC target compilation)

Exact patched v6.18.58 source/preparation provenance is in
`PR1065-MAINE133-INTEGRATION-2026-11-03.md`. Compare the saved integrated compiler
against this change using the same actual Kbuild O3 translation-unit command.
The strict post-fix command adds `-Werror=frame-larger-than=2048` and passes.
Separate zero-threshold warning-only measurements expose the exact post-fix
sizes; these measurement commands do not replace the strict validation.

| Actual NAP function / object | Before | After |
|---|---:|---:|
| Forward diagnostic frame | 2,088 B | 1,960 B |
| Learn diagnostic frame | 2,280 B | 984 B |
| Forward function symbol size | 2,149 B | 2,121 B |
| Learn function symbol size | 1,764 B | 1,436 B |
| Object `size` text total | 3,972 B | 3,612 B |

The object text reduction is **9.06%**; learn function bytes shrink **18.59%**.
This is not a whole-kernel size measurement. Source assembly is assembled by
LCCC and measured with the verified binutils2.47 oracle, not GCC.

## Validation and causal controls

- Rust1.99.1 `fastbuild`, two Cargo jobs, active 4GiB swap, warnings denied.
- All compiler/linker binaries built; library tests **7,836 PASS / zero FAIL /
  ten existing ignores**. Full tests rerun after restoring mutation.
- `cargo clippy --profile fastbuild --all-targets --locked -j2 -- -D warnings` PASS.
- `check_avx256_fp_contracts.py`: **72 build/runtime/frame checks PASS** across
  LCCC, GCC16.2 and Clang23.1.4. x86-64 covers five optimization modes and
  bitwise MIN/MAX cases. i686 executes the original header-free stress fixture
  as a static userspace process at O0/O1/O2/O3/Os, without needing multilib libc.
  Optimized modes retain the strict 2,048-byte limit. This is non-kernel code.
- Saved pre-fix compiler rejects the original private-frame fixture at **4,504
  bytes** under the strict gate: the new frame regression kills the baseline.
- Deliberately restore the four old commutative dispatch flags: the forced
  unit fails with `NaN/signed-zero operand order was reversed: vminps (%rdi),
  %ymm5, %ymm7`. Restore source and rerun all 7,836 units successfully.
- Existing fixed-frame diagnostic matrix **1,747 PASS**; its CI gate now also
  invokes the new AVX contracts (no coverage removed or disabled).
- Targeted AVX differential corpus: **20 PASS / zero FAIL / three existing
  host-multilib skips**. Those three legacy tests need libc-linked i686 host
  support; they are not silently counted as passes. The new i686 fixture is
  independently exercised without that dependency.
- Existing synthetic native mitigation/codegen contracts: **218 PASS** across
  LCCC/GCC/Clang. These are explicitly non-kernel fixtures.
- Benchmark harness unit tests: **20 PASS**; register bounded instrumentation
  sizes and explicit ISA flags for the new exploratory workload.
- gzip `longest_match` O2/x86-64-v3 assembly is **byte-identical** to the saved
  compiler, satisfying the no-increased-stack-memory codegen veto.
- Final canonical boot gate PASS: **31,088 bytes / 1,680 bytes headroom**, 24
  ISA-clean objects and GNU ld flat-image equality. Image SHA256
  `ea8d15014f3ea5edd988963fb49b2d2dd4880656e631d727ff0de46c286bed7d`.
- Restored production compiler SHA256 is identical to the runtime-measured
  candidate: `a51618781358e13f45f48abed83ee0dbd76c0ced0637a256666ad40d14a3a5f3`.

## Paired runtime evidence and limits

Raw samples, compiler/binary/source identities, commands, outputs and bootstrap
intervals are committed in `engineering/evidence/pr1065-avx-contracts-2026-11-03/`.
Canonical runner, 15 randomized paired rounds, two excluded warm-ups, CPU-pinned
VM, O2/x86-64-v3. Every arm's exit status and checksum agrees with GCC16.2:

- Original synthetic `avx_fp_private`: checksum **2088761911016512**; new/before
  median paired time ratio **0.80833**, bootstrap95% **[0.76660, 0.82142]**.
  That is **19.2% less paired median wall time**, not a hardware-counter claim.
- gzip: checksum/reference **15308096299102938189**; new/before **0.99327**,
  bootstrap95% **[0.96838, 1.01396]**. No statistically clear runtime change.
- Absolute performance remains poor on the AVX stress workload: LCCC/GCC
  paired ratio **9.66**, LCCC/Clang **22.85**. Clang's ~11ms duration is below
  the runner's 20ms noise warning threshold. Do not describe this as maximal
  efficiency, vendor parity or a whole-application improvement.
- Hypervisor present, PMU/perf unavailable; no Raptor Lake microarchitectural
  or bare-metal performance claim. No samples were discarded.
- ICX2026.2 and Compiler Explorer provisioning/request attempts are blocked by
  the sandbox allowlist. No older ICX or GCC was substituted. Hosted ICX
  evidence from the previous head does not validate this changeset.

## Next acceptance gates

Refresh the canonical boot gate after the final compiler build, complete the
actual full-kernel link and real objtool/QEMU boot, implement the outstanding
inventoried warning/option semantics, remove resulting kernel diagnostics and
verify hosted CI on the pushed PR head. Until those pass, PR1065 is not ready.

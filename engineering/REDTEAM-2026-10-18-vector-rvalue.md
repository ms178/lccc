# Vector rvalue follow-up audit — 2026-10-18

Base: `aec2a2751a70a59a7cca2b5714c5ad0371f1aa4d` (latest fetched main).
PR #853's earlier broad patch is superseded by merged upstream work. This
replacement retains upstream semantics and carries only two demonstrated
lowering repairs plus regression coverage. The old working tree is preserved
in the session's git stash, not reapplied wholesale over newer implementations.

## Findings and decisions

1. **x86-64 packed call-result subscripting dereferenced value bits.**
   `did(v)[0]` in the recovered lane-one fixture segfaulted at O0/O1/O2.
   Disassembly showed the returned XMM bits copied into a GP register and
   dereferenced as an address. `get_array_base_addr` now asks the existing
   packed-result predicate and uses `get_struct_base_addr` for vector values
   requiring materialization. Component access already uses that materializer.
   No ABI classes change. Ordinary pointer/array/vector objects retain their
   old path. The guard runs in the compiler, not in generated programs.

2. **i686 indirect ExtVector calls spuriously used sret.** The expanded
   fixture failed at O0 (status 2) and segfaulted at O1/O2. Isolating all 15
   shapes reproduced the indirect-call failure individually. Assembly showed
   the caller pushing a hidden pointer and correcting for a callee-pop even
   though the callee used its register carrier. `aggregate_return` now consults
   `i686_ext_vector_ret_carrier` before the record-size fallback, sharing the
   same authoritative table as definition lowering. Non-ExtVector types and
   ExtVectors with no register carrier retain the existing fallback.

3. **Stale tests and claims.** Recovered lane-one/mixed-argument/narrow-math
   fixtures were untracked and therefore absent from CI. They are now tracked
   and wired into the existing lane-one gate, O1 included. Their comments no
   longer claim all FP vectors use SSE or all integer vectors use GPRs. Missing
   compilers fail rather than silently skip. GCC does not implement
   `ext_vector_type`: new corpus sidecars explicitly disable that invalid
   comparison. The dedicated cross gate requires exactly Clang 23.1.3.

## Alternatives rejected

- Changing classification to SSE for every one-lane FP shape, or INTEGER for
  u64x1: contradicted by upstream's pinned Clang measurements; conflates
  `vector_size` with `ext_vector_type` and argument with return position.
- Materializing every vector call globally: changes the packed-result contract
  of unrelated consumers and risks double spills. Repair the address consumer
  using the existing shared representation predicate instead.
- Patching just a named-function call: misses indirect, comma and statement
  expressions. The new matrix exercises all four, dynamic indices and
  exactly-once index/comma side effects.
- Treating all i686 vectors as records: the demonstrated hidden-pointer stack
  imbalance disproves this design. Reusing the carrier table avoids a second
  classification implementation.

## Validation receipts

Rust 1.97.0 installed from SHA-256-verified repository cache tag; GCC 16.2.0
from `harness-cache-1` (archive SHA-256
`4fa6af4f6b29991e31631af18d83964a69caa9daf3f78546d4dc30f39880cd8a`).
8 GiB swap active. All compiler builds use fastbuild.

- `RUSTFLAGS='-D warnings' cargo build --profile fastbuild --bin lccc --bin lccc-ld`: PASS.
- `RUSTFLAGS='-D warnings' cargo clippy --profile fastbuild --all-targets -- -D warnings`: PASS.
- `cargo fmt --check`: PASS.
- `RUSTFLAGS='-D warnings' cargo test --profile fastbuild --lib`: 5162 passed, 0 failed, 10 ignored.
- `check_ext_vector_one_lane_fp.sh`: PASS, including all three recovered gates.
- `check_ext_vector_rvalue_subscript.sh --self-only`: PASS. Fifteen shapes,
  direct/volatile-function-pointer/comma/statement-expression results, every
  live lane; separate provider/caller TUs, O0/O1/O2, x86-64 and i686. The
  freestanding i686 entry needs no multilib libc and propagates main's status.
- Full `run_regression.py --gcc /tmp/gcc-16/bin/gcc-16 -j 4`: 996 passed,
  9 failed, 4 skipped-compare, 14 skipped-run / 1023 total. All nine failures
  are compile failures reporting `cannot find -lgcc` on i686. This is NOT a
  clean full-corpus receipt; environment gaps remain. JSON/logs are local
  scratch artifacts, not a large generated dataset in Git.
- The new cross gate is wired in hosted CI and local fast gates. Default mode
  requires Clang 23.1.3; `--self-only` is explicitly not cross-ABI evidence.

## Boundaries and unproven claims

Clang 23.1.3 and ICX 2026.2 are not installed locally. No fresh local oracle
receipt is claimed for them. GCC cannot validate this attribute. Hosted
results must be checked before merge. ARM/RISC-V extension support is not
introduced here; no runtime validation for those targets is claimed.
Unsupported wide external return carriers remain diagnosed by upstream.
No Intel 14700KF runtime performance measurement, global optimality proof,
exhaustive UB proof, or universal absence of compiler defects is claimed.
These are correctness fixes, not evidence of beating another compiler.

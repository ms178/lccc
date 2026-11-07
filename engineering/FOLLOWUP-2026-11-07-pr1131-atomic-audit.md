# PR 1131: atomic audit and reproducibility (2026-11-07)

## Scope and provenance

The session branch imports PR 1131 commits `bde735b`, `15fa965`, and
`7ca349c` on top of upstream main `0ac52ec951e992847dfb95ff69f650bec7a403ba`.
This is a targeted audit, not a proof of the entire patch or compiler. In
particular, neither the single-threaded cross fixture nor instruction counts
prove a C memory-order mapping or real-hardware performance.

## Reproduced findings and changes

1. The new `atomic_ops_cross.c` is discovered by the native regression runner.
   On x86-64 its 16-byte operations reference libatomic; without a sidecar the
   standalone link fails with undefined `__atomic_*_16` symbols. The hosted
   SSA job's check-run annotation reported precisely this failure. Added
   `atomic_ops_cross.flags` with `-latomic`, using the existing runner's
   library-order handling rather than weakening discovery or replacing the
   actual library with a single-thread stub. The SSA-enabled runner now
   compiles, executes and compares it against GCC 16.2, without a skip.
2. `CHECK(e)` evaluated `e` twice under `GEN+TRACE`. Numerous expressions are
   atomic operations with side effects: before the fix, GCC 16.2's generated
   hash lines differ between `GEN` and `GEN+TRACE`. Evaluate and cast once,
   then hash and trace that saved value. `check_atomic_backends.sh` now checks
   that actual trace rows are present and all 38 group hashes match. The old
   implementation is a negative control: it fails this comparison. The normal
   generator's 38 hashes remain identical to the pre-fix output.
3. With matching Debian i386 runtime and headers installed, the debug-invariance
   gate reproduced two failures: `code16_realmode_encoding.c` used `%rax`, and
   `global_load_snapshot_inline_asm.c` used `%rip`, in i686 mode. Plain `-S`
   did not assemble the invalid instructions, but `-g -S` did. GCC 16.2's
   i686 assembler independently rejected both original fixtures. Select `%eax`
   for the identical ModR/M encoding in the first; use an absolute symbol in
   i686 mode while preserving the memory clobber in the second. Added explicit
   i686 sentinels so sample-position changes cannot lose these checks. This
   explains the reproduced local failure, but the blocked hosted log prevents
   independently confirming that the hosted failure contained the same cases.

## Measurements and validation

Environment: virtualized Intel Xeon @ 2.60 GHz, Linux 6.1.158+, 3.8 GiB
RAM, active 4 GiB `/swapfile`; not the requested i7-14700KF. LCCC was built
with `scripts/build_lccc_fast.sh`: fastbuild, `-O1`, `-j2`, warnings denied.
The pinned `harness-cache-8` restored Rust 1.99.1, GCC 16.2.0, Clang 23.1.5,
GDB 18.1, binutils 2.47, mold 3.0.0 and QEMU 11.1.3. GCC 12.2 is the system
default, not the GCC oracle. ICX/ICC were not available. Compiler Explorer
requests failed because its host is outside this sandbox's network allowlist.

* `CCC_VALIDATE_SSA=1 python3 tests/regression/run_regression.py --lccc target/fastbuild/lccc --gcc gcc-16 --filter atomic_ops_cross -v`: **PASS**, including GCC comparison. Compile-only: **PASS**.
* Native LCCC `-O0 -O1 -O2 -Os -O3`, with `-latomic`: **5/5 PASS**.
* `python3 tests/regression/check_switch_cross_execute.py --qemu-dir "$HOME/.cache/lccc/agent-toolchains/bin" --only atomic_ops_cross --keep-going`: **10/10 PASS**, both targets at the same five optimization levels, using LCCC's assembler. RV64 uses the fixture's sequential, non-atomic 16-byte helper stubs, not a real libatomic interoperability test.
* `bash tests/regression/check_atomic_backends.sh`: **PASS**, including the generator/trace contract.
* GCC 16.2 `-Wall -Wextra -Werror`: **PASS** on the fixture. Clang 23.1.5: **PASS** with `-Wno-atomic-alignment`; without it Clang correctly emits the explicit 16-byte non-lock-free performance warning, so this is not a blanket zero-warning claim.
* `check_debug_info_invariance.sh`: after provisioning i386 headers and fixing the two fixtures, **345 PASS**, 4 skipped. Before those fixes, the same environment failed on both i686 inputs. The previous 160-pass/189-skip result was not adequate coverage.
* GCC 16.2 and LCCC executed both corrected fixtures in native 64-bit mode with and without `-g` at `-O2`; GCC did the same in 32-bit mode. LCCC's i686 binaries passed 20 runs (two fixtures, five optimization levels, with/without `-g`), with explicit `-L` paths to the verified cached GCC 16.2 32-bit `libgcc` and `libgcc_s`. The default host search did not locate these cached libraries.
* `git diff --check origin/main HEAD`: **PASS** at the audited revision.

No PMU counters, Raptor Lake timing, contention measurements, performance
superiority or complete architecture coverage are claimed. QEMU validates
functional execution, not target cycle counts. The remote Compiler Explorer
oracle was attempted and failed; GCC/Clang version identities above refer to
local pinned tools, not the system default.

## Remaining high-priority work

1. Verify the corrected `-g` gate on hosted CI. Four local cases are still
   skipped and must not be interpreted as coverage. The old hosted job log
   was inaccessible here, so the two locally reproduced invalid fixtures are
   not by themselves proof of the exact hosted failure.
2. Run the complete fast local gate on the *final unchanged tree*, then hosted
   CI, including the assertion-enabled and SSA regression corpora. A passing
   focused test is not a substitute. Treat missing toolchains and skipped
   gates as unmeasured; do not close PR 1131 while replacement CI is red or
   pending.
3. Exercise real AArch64 and RV64 libatomic interoperability in both
   directions, including 16-byte compare-exchange failure writeback,
   contention, aligned objects and ABI register pressure. The test stub only
   checks sequential lowering. Model-check acquire/release/seq-cst mappings
   and mixed-size interactions against the architecture and C memory models;
   run multicore litmus/stress tests on actual hardware.
4. Audit all added register-allocation and peephole paths, including aliasing
   of operand homes and scratch registers, call/clobber boundaries, floating
   argument staging, frame/unwind metadata, and all exits of the RISC-V frame
   removal. Add minimal executable cross-target and negative-control tests for
   each newly identified counterexample. Existing tests do not establish that
   every path or ordering was audited.
5. Run the real gzip, zlib-ng and expat workloads and the broader corpus;
   compare assembly and statistically controlled native measurements against
   GCC 16.2, Clang 23.1.5 and ICX 2026.2 on verified i7-14700KF P-cores and
   E-cores. Record toolchain versions, affinity, thermal state and available
   PMU counters. Do not infer cycle gains from QEMU or static instruction count.
6. Provision the missing cross GNU tools, multilib, ICX and any other required
   oracle; in this sandbox only the explicitly allowed GitHub/npm/PyPI hosts
   are reachable. Restore cached payloads only with verified provenance and
   hashes. `-Watomic-alignment` is a useful Clang diagnostic and should not be
   silently disabled globally.

Session snapshots were published by `scripts/lccc-snapshot.sh` with an
explicit `UNGATED` ledger entry. The patch applies cleanly to the recorded
base, but this is not a CI or production-readiness attestation.

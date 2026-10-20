# Volatile load-to-ALU fusion: adversarial follow-up

## Scope and decision

The implementation changes x86-64 text peepholes only. It does not introduce
new i686, AArch64 or RISC-V lowering. Extending the exception to additional
volatile shapes, changing register allocation, or enabling loop unrolling is
not justified by this patch's evidence. Volatile is not atomic synchronization.

The production delta is intentionally small: tighten volatile adjacency and
make the exceptional pin-transfer contract explicit. The bulk of the follow-up
is executable safety coverage. No claim of universal correctness or optimality
is made; finite tests cannot establish either.

## Re-audit of the review

1. **Confirmed oracle defect:** command substitutions inside `[` discarded
   execution statuses. Replace them with bounded, separate executions and
   file comparison. The hermetic oracle test covers eight success/failure,
   mismatch and timeout combinations, including equal-output failures.
2. **Confirmed coverage weakness:** a segmented-gap negative case also exercises
   the old segment guard. Add an unsegmented volatile/nonvolatile pair.
   Removing the volatile adjacency term was actually compiled and tested: the
   targeted negative case failed. Restore the source before final checks.
3. **Coverage, not demonstrated miscompilation:** exercise all eight direct ALU
   consumers at both widths over four address shapes (64 cases), sign-extension
   width boundaries, forbidden shapes, live temporaries, barriers and pins.
   Rejected transformations must preserve text and all metadata fields.
4. **Corrected review premise:** final assembly drops volatile markers and is
   not a lossless internal IR serialization. Test repeated pass execution with
   metadata intact; for a second full pipeline entry, restore real markers.
   Nested markers and two-read order are tested through the production pipeline.
5. **Intentional shared grammar:** retain symbolic segmented operands for MOVBE
   and zero-extension compares, rather than duplicate operand parsers. Add
   affected-consumer coverage, including MOVBE feature gating and pinned input
   rejection; correct the stale parser comment.
6. **Conservative boundary policy:** the ordinary walker skips more than debug
   attribution. Volatile fusion now uses `debug_transparent`, rejecting CFI,
   alignment and pinned directives without changing nonvolatile behavior.
   Deleted lines and unpinned debug attribution remain transparent.
7. **Additional pre-existing harness defect:** shared-object assembly failure
   could be swallowed and followed by a successful model-selection message.
   Record a failure and discard partial output instead.

## Implementation tradeoffs

- Reuse existing opcode, exact-width and liveness proofs. Do not create a new
  volatile optimizer or broaden the accepted volatile shapes.
- Retire a pinned load only after legality checks, then reclassify and repin the
  one surviving read. Add debug assertions at this exceptional editing boundary.
- Keep metadata equality/debug derives test-only: exact rejection assertions add
  no production code or metadata footprint.
- Keep fault-injection tests compiler-independent and run them in CI before
  compiler-dependent TLS contracts.
- Exercise changing unsigned volatile values at both widths and nonzero TLS
  addends. Unsigned wraparound is defined; compare candidate/control binaries
  with GCC and independently with Clang. Assembly assertions separately detect
  the intended optimization, rather than relying on output parity for access
  count.

## Reproducible local checks

Restored SHA-256-checked cached Rust 1.97.0, GCC 16.2.0 and Clang 23.1.3 from the
repository's cache tags. A 1 GiB swap file was active. Built all compiler binaries,
including lccc and lccc-ld, through `scripts/build_lccc_fast.sh`; all Cargo checks
used the fastbuild profile and warnings were denied. The full library test suite
reported **5,324 passed, 0 failed, 10 ignored**. The existing ignored tests were
not silently counted as passes. All-target Clippy with `-D warnings`, formatting,
the directive-walker guard (36 witnesses), shell syntax, eight oracle fault
cases and the complete TLS contract script passed.

GCC 16.2.0 and Clang 23.1.3 both compiled the new C fixture with
`-O3 -Wall -Wextra -Werror -fno-pic -no-pie`; their output matched LCCC. The TLS
contract script used GCC 16.2.0, including its five-optimization-level by
four-code-model matrix, shared-object/dlopen checks, five hot-loop trip counts,
and changing-value candidate/control comparisons.

Commands (set PATH to the restored Rust installation first):

```sh
scripts/build_lccc_fast.sh
export RUSTFLAGS="$(cat target/lccc-rustflags)"
cargo test --profile fastbuild --locked -j2 --lib -- --test-threads=2
cargo clippy --profile fastbuild --locked -j1 --all-targets -- -D warnings
cargo fmt --all -- --check
bash tests/regression/check_tls_runtime_oracle.sh
GCC=/path/to/gcc-16 bash tests/regression/check_tls_model_selection.sh
python3 scripts/check_peephole_directive_step.py
```

Clippy was run serially after two concurrent frontends caused swap pressure;
this changes resource scheduling, not which targets or diagnostics are checked.
ICX 2026.2 was not available in the discovered cache tags. The environment's
outbound-host policy does not permit the Intel PPA, so no ICX result is claimed.
No cross-architecture runtime testing or sanitizer proof is claimed.

## Performance evidence and limits

Disassembly of `tls_hot_loop` confirms the eligible loop shrinks from five to
four instructions and saves three loop-body bytes. GCC and Clang also select a
segmented memory-source add. A folded memory instruction need not execute fewer
micro-operations than the standalone load and register add.

After builds finished, nine rotating-order serial runs per compiler/control,
pinned to virtual CPU 0, each executed one billion loop iterations. All outputs
and exit statuses agreed. Median subprocess wall/CPU seconds:

| Binary | Wall | CPU |
| --- | ---: | ---: |
| LCCC folded | 0.373011 | 0.371481 |
| LCCC fusion disabled | 0.373619 | 0.372091 |
| GCC 16.2.0 | 0.375076 | 0.373452 |
| Clang 23.1.3 | 0.360570 | 0.359026 |

These virtualized-host measurements do **not** establish a significant LCCC
speedup or superiority over Clang. Clang's loop is unrolled, so it is not an
isolated fusion comparison. Earlier samples overlapped memory-heavy checking
and were rejected as performance evidence. No ISA policy or profitability
heuristic is tuned to that noise. The defensible benefit here is the verified
instruction/temporary reduction without a demonstrated material regression in
this limited workload, not a universal cycle-count guarantee.

## Delivery

The cumulative main-relative patch is saved as `ms178-1.patch` in the repository
root (intentionally ignored), with numbered checkpoints under
`work/audit/snapshots/`. Build products and downloaded toolchains remain ignored.
Remote CI status is reported separately because this document is not evidence
that a future remote run passed.

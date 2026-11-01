# Unified PR audit — 2026-11-01

## Scope and acceptance status

The requested consolidation covers #1050 (`a62b7d708`), #1051 (updated to
`2bb4e69e2`), and #1056, on `arena/b673aa4d-lccc`. Both other heads are ancestors
of this branch. `git fetch origin main` and `git rebase --rebase-merges
origin/main` were run against main `013983d6a`. A newly opened #1060 is outside
this originally enumerated set; it has not been folded in or represented as
audited here.

**This is an in-progress audit, not a production-readiness certificate.** The
previous #1056 hosted run was red. Full local gates and a new hosted run are
required before acceptance. Historical benchmark and component reports are
not evidence that every program on the unified tree is correct or faster.
No independent expert approval is asserted.

## Defects exposed by integration and adversarial verification

### 1. Literal SIMD lanes were treated as addresses on the SSE2 multiply path

Generic literal-lane SLP eligibility was unsound: the SSE2 dword multiply
consumer interprets a non-homed vector argument as a memory address, whereas
some other consumers know how to synthesize constants. The real SHA-256
regression faulted while dereferencing the literal 1664525. The fix admits
literals by **consumer capability**, retains the all-ones compare idiom, and
does not pretend that an unsupported literal operand is a spill address.

`slp_dword_constants.c` and `check_slp_dword_constants.sh` exercise hoisted
loads, require the intended vector instruction, compare outputs, and retain
the no-SLP arm. An earlier fixture was rejected as vacuous and a helper named
like a compiler builtin was renamed rather than relying on builtin resolution.
The warning-denied build, constant gate, BB-SLP v5 and reassociation gates pass.
SHA-256 checksum: `054db5f638d89d8b` on baseline x86-64 and v3.

### 2. Inliner ABI homes need provenance and shape, not alloca order

The merged inliner resolves homes by parameter identity/data flow, validates
ABI shape, and allocates collision-free fresh IDs. A same-typed leading local
is not identity proof. Tests cover a leading local, mismatched ABI-home shape,
and stale `next_value_id`. The ABI differential gate passes 12 rows
(O1/O2/O3/Os across i128 switches, Float128 conversions and SSE1-only vec8),
with an explicit assertion that the i128 parameter is actually inlined.

A tentative aggregate-ParamRef change was withdrawn: the crashing `bits` and
`mbits` helpers contain **no ParamRef** in their initial IR. Its small runtime
probe also passed on the old compiler, so it was not credited as a regression
or used to justify a speculative production change.

### 3. Destructive phi coalescing assigned a register to an unmaterialized Alloca

The real `vector_small_packed_wrappers_m32.c` failed with SIGSEGV at O2, while
GCC 16.2 and the inline-disabled build printed
`small vector packed wrappers a23f76a3732b5dd3`. Disabling assembly peepholes did
not repair it. Skipping `bits,mbits` or disabling phi coalescing did repair it;
skipping only `mix` or the MMX wrappers did not.

A ptrace breakpoint on `ret` established that call 1 had a valid sret buffer,
but call 2 passed zero. The false predecessor passed EBP before any defining
LEA; the return-buffer LEA appeared only after the call. The callee's `ret $4`
and the caller's reserve-32/add-28 stack cleanup were correct, so changing
cleanup would have attacked the wrong boundary.

`Alloca` produces a rematerializable frame address, **not an emitted register
definition**. Phi-home inheritance bypassed ordinary eligibility and gave
that address the phi's register. Uses before the latch copy read an
uninitialized register. The shared candidate detector now rejects an actual
Alloca source definition. It does not disable phi coalescing globally or
change ordinary memory operand handling. A materialized Copy of an Alloca
remains eligible; the Part-2 chain's actual source definition, rather than its
underlying root, is what matters.

Rust tests pin both the refusal and materialized-pointer positive controls.
The rebuilt compiler passes the real m32 fixture with the GCC checksum above.
The diagnostic trace harness also passes. This is a shared allocator fix,
not an i686-only operand-order workaround; cross-target execution is still a
separate validation obligation.

### 4. Merged simplify tests had an obsolete call signature

Nine test/wrapper calls lacked the thirteenth argument of
`try_simplify_with_types`. The repairs were compiled in the first full unified
library run: **7,426 passed, 10 ignored, 0 failed**. The latest #1051 correction
was merged as well; its overlapping repairs produced no duplicate arguments.

### 5. Bounded SIMD metadata validation was duplicated and truncated I128

Four new internal-control decoders duplicated constant/range handling.
`IrConst::to_i64` truncates I128, allowing a malformed value with high bits set
to masquerade as a valid shuffle/control byte. A shared checked decoder now
converts through I128 and checked I64 conversion before enforcing the exact
range. Missing operands, SSA values, floating constants, out-of-range values
and high-bit I128 aliases must fail closed. Unit tests enumerate all valid
controls and those invalid classes. This does not weaken target-feature or
operand-count assertions.

Deduplication removes seven production panic/expect sites. Relative to the
fetched main census, production panic sites fall **881 → 877**, hot panic
sites stay 101, unsafe sites stay 25, long functions (>300 lines) fall
233 → 232, unique knobs stay 549 and undocumented knobs fall 264 → 262.
The integrated baseline is regenerated against that upstream baseline;
there is no ratchet increase relative to main. The internal tail-call marker
is no longer mislabeled a user configuration knob.

## Validation receipts so far

All compiler builds use the fastbuild profile, two Cargo jobs, warnings denied,
and the active 4 GiB swap. Heavy builds are serial.

- Warning-denied compiler/linker build: PASS.
- SLP literal constants, BB-SLP v5, latency reassociation: PASS.
- Real i686 small packed-wrapper fixture: PASS after the allocator fix.
- Inliner ABI/home gate (12 differential rows plus actual-inline coverage): PASS.
- PIE executable/ET_DYN and struct-by-value execution gate: PASS.
- LightPixel and joint-pixel differentials: each 6/6 configurations PASS
  (SSE2/SSE4.1/AVX2 × O2/O3).
- First unified Rust library run: 7,426 passed / 10 ignored / 0 failed.
- Warning-denied all-target Clippy: PASS.
- Python SIMD harness suite: 22/22 PASS.
- Census, documentation links and generated CI lane-plan checks: PASS.

These receipts are staged validation, not a claim that a later tree edit was
already exercised. The subsequent complete local run must validate the final
materialized-copy positive controls and bounded-control tests on one unchanged
tree. Hosted checks, complete regression/corpus results, cross execution and
fresh unified timing receipts remain outstanding at the time of this note.

## Remaining boundaries and limits

The follow-up documents remain evidence inventories, not completed checklists.
In particular, exact Chromium ClampTo8 fastguard recognition, a build of the
comparison LLVM PR, broad browser relevance, external ICX execution and
architecture-specific runtime coverage must not be inferred from an integer
pixel microbenchmark. The existing codegen-mode follow-up records its landed
non-PIC fix and its GCC/kill-switch runtime regression separately from its
remaining profitability questions. No unsupported memory-width, alignment,
strict-FP, overflow or whole-browser performance claim is made here.

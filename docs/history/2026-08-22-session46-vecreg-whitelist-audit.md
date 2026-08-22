# Session 46: fail-closed vecreg whitelist audit

**Date:** 2026-08-22
**Base:** `c6c09274029e0f6a96ef883fcc2066b3d0968275` (PR #182)
**Backlog item:** RA-21

## Rebase and formatting

The implementation was rebased from PR #181 onto PR #182, whose implementation
commit `6a96900b` reformatted every Rust source. The conflict in the SSE binary
match was resolved semantically: PMOVZX remains removed from the binary arm and
is retained in its new unary arm. All three Rust files changed by this patch
were then formatted with rustfmt 1.9.0 using the 2024 Style Guide edition
(`cargo fmt --all -- --style-edition 2024`); a targeted 2024-style check passes.

## Scope and decision

This session audited `collect_vecreg_candidates`, the x86 intrinsic lowering
contracts it relies on, and the block-boundary register flush. The bounded
RA-21 change was accepted because it has a local safety proof, a kill switch,
a generated-code improvement, and a statistically resolved non-PMU screen.
The generic 128-bit FP and EVEX families remain closed: some of those emitters
record transient `vec_live_regs` but do not honor an allocator-assigned
destination in the `sse_store_dest` protocol. Opening them wholesale would let
stack layout omit a home that their fallback emitter still addresses.

## Audit findings and fixes

### 1. Intrinsic-level admission confused scalar/immediate values with vectors

The old whitelist returned one Boolean per operation. If an operation was
accepted, every `Operand::Value` argument was counted as a cache-aware vector
read. That was wrong for instructions with trailing scalar or immediate
operands, including shifts, shuffles, AES key generation, CLMUL, GFNI affine
operations, blends, and inserts. A candidate ever reaching one of those roles
could be assigned an XMM home and later be interpreted as a scalar or address.

The whitelist now returns `Option<usize>`: the exact number of leading vector
arguments. The groups are explicit and fail closed:

- zero-input full-width producers;
- unary vectors, including immediate forms and scalar inserts;
- ordinary two-vector operations, optionally followed by an immediate;
- three-vector blend/VNNI operations.

Unknown operations receive no admission. The new classification also admits
the post-whitelist SSE2 binary family (`paddusb`, signed/unsigned saturating
word operations, averages, compares, min/max, high multiply, qword add/sub,
and dword/qword unpacks), plus audited insert and zero producers. Every admitted
operation loads vector operands through `sse_load_arg` and writes its complete
128-bit result through `sse_store_dest`. The allocator pool remains XMM3-XMM7,
away from the XMM0-XMM2 implicit scratch set.

### 2. Local values were falsely classified as live out

`count_value_uses` uses both canonical operand visitors, but the per-block
counter duplicated only part of that logic. It omitted
`Intrinsic::dest_ptr`, making every local vector result appear to have an
extra use outside its block. The end-of-block flush then wrote XMM3-XMM7 homes
back to dead stack slots. Conversely, the handwritten code double-counted
atomic pointer operands, which could hide a real use in another block.

Per-block counting now invokes the same `for_each_value_use_in_instruction`
visitor as the global counter. On the one-block saturation chain this removes
two false live-out stores while preserving the kill-switch stack path. Actual
cross-block differences are still conservative; replacing count comparison
with exact CFG live-out sets is separate work.

### 3. The audit exposed a valid-source compiler panic

`Pmovzxbw128` and `Pmovzxwd128` are unary SSE4.1 widening operations, but their
x86 dispatch was grouped with binary operations. Valid
`_mm_cvtepu8_epi16(x)` reached `emit_sse_binary_128`, asserted that two
arguments existed, and produced an internal compiler error. They now have a
unary lowering (`pmovzxbw/pmovzxwd %xmm0, %xmm0`) using the same register-aware
load/store protocol. The runtime regression exercises both operations.

## Generated-code evidence

`simd_vecreg_new_ops.c::sat_chain_store` deliberately uses two packed values
more than once. The default path and `CCC_NO_VECREG=1` control are built by the
same fastbuild compiler with `-O2 -msse4.1`.

| Metric | register path | stack control | delta |
|---|---:|---:|---:|
| body instructions | 24 | 25 | -1 |
| vector stack accesses | 1 | 6 | -5 |
| intermediate vector loads | 0 | 3 | -3 |

The treatment keeps the multi-use results in XMM3/XMM4. The one remaining
stack vector access is an existing final-result store before the output store;
it is outside RA-21 rather than hidden from the count.

## Runtime screen

`benchmark_vecreg_new_ops_ab.py` ran 32 randomized pairs pinned to CPU 0, with
100,000,000 kernel iterations per sample. Treatment and control are the same
fastbuild compiler; only `CCC_NO_VECREG=1` differs. There is no PMU in this
environment, so this is VM wall-time screening only.

| Result | Value |
|---|---:|
| paired register/stack geometric mean | **0.940994** |
| paired bootstrap 95% interval | **0.918250 .. 0.962704** |
| register-path wins | **31 / 32** |
| treatment median | 158,708,189 ns |
| control median | 162,600,572.5 ns |

Raw samples and metadata are in
`/home/user/artifacts/ra21-vecreg-new-ops-timing.json`.

## GCC / Clang / ICC / ICX oracle

The tracked benchmark was submitted through `scripts/codegen_oracle.py` with
`-O3 -msse4.1`. The complete manifest, assembly, and report are under
`/home/user/artifacts/ra21-oracle`.

| Compiler | static instructions | loads | stores | spills |
|---|---:|---:|---:|---:|
| LCCC treatment | 58 | 12 | 5 | 8 |
| GCC 16.2 | 19 | 3 | 1 | 0 |
| Clang 22.1 | 11 | 0 | 1 | 0 |
| ICC 2021.10 | 16 | 5 | 1 | 0 |
| latest ICX | 23 | 4 | 1 | 0 |

RA-21 removes the stack round trips it owns, but the oracle exposes a larger,
separate loop-invariant/dead-store gap: competitors move or collapse the
iteration-invariant packed chain and repeated overwrite. That belongs in LICM
and non-escaping dead-store work, not in an unsafe expansion of vecreg
admission.

## Red-team checklist

- **Widths and bounds:** admission is restricted to complete 16-byte results;
  256/512-bit homes cannot enter the 128-bit pool.
- **Argument roles:** only exact vector-prefix indices count as vector reads;
  scalar, immediate, address, store-payload, and unknown roles poison a value.
- **RMW:** same-destination recurrence detection only observes an audited
  vector argument, never a scalar/immediate coincidence.
- **Alignment/volatility:** volatile, semantic-volatile, and over-16-aligned
  allocas remain excluded.
- **Calls/CFG:** call-spanning intervals remain excluded; non-intrinsic and
  terminator uses remain fail-closed. The canonical block-use visitor repairs
  local/live-out accounting without relaxing those rules.
- **Implicit registers:** admitted new binary operations touch XMM0/XMM1 only;
  the home pool is XMM3-XMM7. Operations using XMM2 remain separated from it.
- **Malformed input:** widening conversion lowering no longer assumes a second
  argument; no unchecked indexing was added to the candidate scan.
- **Compile-time complexity:** the scan remains linear in IR size and uses
  hash-set/map membership; no per-value rescan was introduced.

## Tests and reproducibility

- `tests/regression/simd_vecreg_new_ops.c`: scalar-reference runtime for the
  saturation/average/qword chain and unary PMOVZX operations.
- `tests/regression/check_vecreg_new_ops_codegen.sh`: exact default/control
  structural gate, including XMM homes, stack-access delta, and instruction
  count.
- `tests/benchmark/programs/vecreg_new_ops.c`: deterministic timed kernel.
- `scripts/benchmark_vecreg_new_ops_ab.py`: randomized paired screen with raw
  samples and a bootstrap interval.

Validation completed and repeated after the PR #182 formatting rebase:

- mandatory 8 GiB `/swapfile` active;
- fastbuild (`-O1`, `-j2`) clean;
- targeted rustfmt 1.9.0 2024-style check: clean for all changed Rust files;
- `cargo test --profile fastbuild --lib --locked -j2`: **1045 passed, 0
  failed, 6 ignored**;
- differential regression runner: **374 passed, 0 failed, 7 GCC-oracle
  skips, 1 host i386-loader skip** across 382 tests;
- intrinsic differential suite: **3 passed, 0 failed** (128/256/512-bit);
- focused default/control runtime and all vector structural gates pass;
- pinned GNU gzip 1.14: **30/30 upstream tests** in both treatment and
  `CCC_NO_VECREG=1` builds plus exact deterministic workload outputs; five-round
  VM screen is neutral/unresolved (treatment/control geometric mean 1.000983,
  individual cases 0.995873..1.006370), so no whole-project speed claim;
  report and raw samples are under `/home/user/artifacts/ra21-gzip`;
- the aggregate shell structural suite reports 390 pass, 1 host skip, and two
  pre-existing performance-check failures (`check_load_widen_cast_no_relay`,
  `check_tier2_graph_default`). Both fail identically with an independently
  built clean `5279232f` compiler; they are not hidden as patch regressions.

## Follow-up work

1. Replace duplicated intrinsic lists with one authoritative signature table
   containing result width, vector/scalar/address/immediate roles, destination
   access mode, alignment, and implicit-register clobbers.
2. Replace the block/global count heuristic with exact CFG live-out sets, then
   elide stack homes for register-only allocas that cannot escape.
3. Address the oracle's invariant-chain/repeated-store gap in LICM and DSE with
   alias and volatility proofs; do not encode it as vecreg special cases.
4. Add allocator-aware destination handling before considering generic FP,
   EVEX, or 64-byte AVX-512 vecreg admission.
5. Validate the VM result on the requested i7-14700KF with fixed P-core
   affinity/governor and PMU counters.

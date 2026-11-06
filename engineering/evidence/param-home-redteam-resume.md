# PR follow-up red-team record — resumed parameter-home index (2026-11-06)

Branch `arena/314532f0-lccc`, based on `origin/main 55127d7`
(`Merge pull request #1115 from ms178/arena/ea36223e-lccc`).

## 0. Why this PR exists: PR #1115 was merged from a STALE head

`git cat-file -p 55127d7` shows parents `90feb0b4` and `c2f0e93`.  The second
parent is the PR head at merge time.  The final validation work of the previous
session was never pushed, so the merged `main` contains the `c2f0e93` code,
which has:

* the `ParamAllocaIndex` O(1) first-match lookup (good),
* **but NOT** the resolved-chain guard, the `chain_steps` witness, the
  four-ISA execution harness, the compiled layout test, the i686 single-read
  kill switch, or the strict-Clippy fix.

`git merge-base --is-ancestor c1f532caa 55127d7` -> false; the stage-11 objects
are absent locally and `refs/heads/arena/ea36223e-lccc` still points at
`c2f0e93`.  This PR therefore re-derives that work from the merged tree and
goes further.  Nothing below is inherited from an unpushed head.

## 1. Defect reproduced BEFORE any code change

`ParamHomeStats` gained `chain_steps` and two adversarial tests were added to
the UNMODIFIED merged code.  The instrumented build counted:

```
duplicate_allocas_do_not_rescan_resolved_chains: FAILED
  102656 chain steps for 512 params: a resolved chain was re-walked
chain_steps_are_linear_in_every_duplicate_shape: FAILED
test result: FAILED. 16 passed; 2 failed; RC=101
```

Two disjoint chains (256 ordinals each), one deliberately left unresolved so
the `unresolved == 0` early exit cannot hide the cost, and 400 duplicate
`Alloca`s for the resolved home: `256 + 400*256 = 102656` links instead of
`<= 512`.  `instructions_examined` stayed linear throughout — the witness was
blind to link traversal, which is exactly why this defect survived the first
PR's review.

## 2. The fix AND a strictly better design

### 2.1 O(1) resolved-chain guard (correctness / complexity)

Chains are disjoint (one `head` per `Value`) and filled atomically, so
`homes[first].is_some()` is an exact "this chain is already resolved" flag.
After the fix the same test reports `chain_steps <= 512` and all 20 index
tests pass.

### 2.2 Seeded single-pass RESUME (new; better than the previous design)

The old `aligned_homes` cleared its work on failure and `general_homes`
re-walked the ENTIRE entry block, giving
`instructions_examined <= params + entry_instructions`.  The new
`aligned_probe` fills `homes[0..k]` BY INDEX and keeps them on failure;
`general_homes_from(..., start = k)` resumes at `k` and builds the map only
over `wants[k..n]`.

Exactness of the resume: `wants` is strictly increasing (checked from `wants`
alone, before any instruction is examined).  For an unresolved ordinal
`i >= k`, every instruction `j < k` has `dest == wants[j] < wants[k] <=
wants[i]`, so it cannot be the first match for `i`.  A late `Alloca` repeating
an earlier home id is absent from the restricted map and is ignored for free.

New bound: `instructions_examined <= entry_instructions + 1`, pinned EXACTLY
by `failed_aligned_probe_resumes_without_reexamining_the_prefix`
(`entry_instructions + 1` instead of the old `FAIL_AT + 1 +
entry_instructions`).  The single `+1` is the boundary instruction seen once by
the probe and once by the resumed walk.  Removing it would require a second
resolution entry point plus a closure over five mutable borrows to save ONE
instruction examination out of thousands: complexity churn, deliberately
rejected.

`wants.windows(2).all(...)` decides the mode from the contiguous `wants` array
with ZERO instruction examinations, so non-strict (duplicate) layouts never
pay for a doomed probe.  Fusing it into the ordinal loop was considered and
rejected: for a non-strict layout it would examine `k` instructions before
giving up, which is strictly worse.

Alternatives considered and rejected, with reasons:

| alternative | verdict |
| --- | --- |
| Delete the now-always-true `if homes[o].is_none()` | Rejected: `fastbuild` inherits `release`, so `debug_assert!` never runs even in `cargo test --profile fastbuild`; the cheap branch is the ONLY runtime verification of the non-local chain-disjointness invariant on the shipped profile. Cost is <= n predictable branches. |
| Binary-search `wants[k..n]` instead of a hash map on the strict resume path | Rejected: O(log n) per instruction vs O(1) hash, saves two allocations but adds a third code path and its own exactness proof; no measured win. |
| Hand the boundary `Alloca` to the general resolver to remove the `+1` | Rejected: saves one examination out of thousands for a second resolution entry point. |
| Claim a general compiler speed-up vs main | Rejected: measured (below), the hot aligned path is unchanged and normal C shapes are within noise. |

## 3. Line-by-line safety audit of the changed functions

* `build_homes`: `n == 0` returns before any allocation; `with_capacity(n)` +
  `resize(n, None)` is ONE allocation and cannot reallocate; `strict` uses
  only `wants`; `resume < n` is `debug_assert`ed.
* `aligned_probe`: `blocks.first()` guarded, `instructions.len() < n` guarded,
  `block.instructions[i]` bounds-checked, `homes[i]` bounds-checked into a
  slice of length `n`; `*dest != wants[i]` compares two `Value`s (no pointer
  dereference); no arithmetic that can overflow.
* `general_homes_from`: `head` only ever holds ordinals in `start..n`;
  `next` is `usize::MAX`-terminated; every `homes[o]` / `next[o]` access is
  bounds-checked; `unresolved = n - start` with `saturating_sub`;
  `block.instructions[start..]` is safe because `start < n <= len` on the
  probe path and `start == 0` otherwise.
* No `unsafe`, no raw pointers, no `NonNull`/`MaybeUninit`, no atomics, no
  locks, no thread-local state, no `Send`/`Sync` coupling.  `NoStats` is a ZST,
  so monomorphization folds all accounting away in `build()`.
* i686: `std::env::var_os` is safe in edition 2024 (only `set_var` is
  `unsafe`); the flag is now read ONCE per function and threaded as an
  explicit `bool`, removing O(params) redundant global reads and the
  context-safety hazard of different parameters observing different values.

## 4. Validation results (local; hosted CI is the authoritative gate)

Per the standing instruction, LOCAL CI WAS NOT RUN.  Only targeted local
tests plus the project's own single contract gate were executed.  `ci_local.sh`
was deliberately not invoked.

| check | result |
| --- | --- |
| defect reproduction on merged main | RC=101, 16 pass / 2 fail, 102656 chain steps |
| `cargo test --lib --profile fastbuild param_alloca_index_tests` | **20 passed / 0 failed** |
| exhaustive resume shapes (`resumed_probe_matches_the_oracle_over_all_late_failure_shapes`) | 360 cases, all vs `reference_full_scan` |
| `cargo test --lib --profile fastbuild` (full) | **8135 passed / 0 failed / 10 ignored** |
| `cargo clippy --profile fastbuild --all-targets -- -D warnings` | **RC=0** (one `identity_op` found and fixed; one earlier `manual_repeat_n` class avoided) |
| `cargo fmt --all -- --check` / `git diff --check` | RC=0 / RC=0 |
| `cargo check --profile fastbuild --all-targets` | RC=0 |
| `check_param_home_contracts.py` (checks 1..13) | **13/13 PASS** |
| `check_param_home_contracts.py --self-test` | **43 non-vacuous rejection paths** |
| `check_param_home_cross_execute.py --self-test` | good / bad / wrong-checksum / SPARC e_machine all correctly classified |
| `check_param_home_cross_execute.py` full | **28/28 EXECUTED** (4 ISAs, 2 fixtures, 3 opt levels, GCC 16.2 + Clang 23.1.5 host oracles) |
| `check_switch_cross_execute.py --only <two fixtures>` | **16/16** (aarch64 + riscv64, `-O0/-O1/-O2/-Os`) |
| `test_check_switch_cross_execute.py` | 12/12 unittest OK |
| `param_home_resume_scaling.py --self-test` | RC=0, asm identical |
| `param_home_resume_scaling.py` full | **12/12 byte-identical assembly** |
| `readelf -h` on the four driver objects | ELF64 X86-64 / ELF32 Intel 80386 / AArch64 / RISC-V |
| `scripts/audit_census.py --check` | PASS, no metric grew |

### 4.1 Scaling (OLD = pristine `origin/main` 55127d7, NEW = this head)

All 12 pairs emit byte-identical assembly, so this is a pure compiler-speed
comparison.  Flat `s += pI;` bodies, `-w -O2 -S`, `CCC_TIME_PHASES=1`.

| shape | n | OLD codegen s | NEW codegen s | ratio |
| --- | ---: | ---: | ---: | ---: |
| aligned | 500 | 0.067 | 0.068 | 0.99x |
| aligned | 1000 | 0.147 | 0.141 | 1.04x |
| aligned | 2000 | 0.269 | 0.277 | 0.97x |
| aligned | 4000 | 0.657 | 0.698 | 0.94x |
| aligned | 8000 | 2.261 | 2.130 | 1.06x |
| aligned | 16000 | 6.909 | 6.243 | 1.11x |
| shifted | 500 | 0.075 | 0.079 | 0.95x |
| shifted | 1000 | 0.159 | 0.152 | 1.05x |
| shifted | 2000 | 0.326 | 0.310 | 1.05x |
| shifted | 4000 | 1.019 | 0.999 | 1.02x |
| shifted | 8000 | 2.854 | 2.582 | 1.11x |
| shifted | 16000 | 8.464 | 8.545 | 0.99x |

**Honest reading:** the hot paths are unchanged, and these realistic shapes
are within noise (0.94x..1.11x).  NO general compiler speed-up is claimed.
The improvements are structural and machine-independent: a proven 200x
reduction of chain-link traversals on adversarial IR (102656 -> 512), a proven
reduction of instruction examinations from `params + entry` to `entry + 1` on
late-failure shifted layouts, and one environment read per function instead of
`params`.  Wall-clock ratios are not portable; `ParamHomeStats` is the proof.

## 5. Tool availability (honest)

* Restored from release tag `harness-cache-8` (12 archives verified):
  GCC 16.2.0, Clang 23.1.5, GDB 18.1, binutils 2.47, mold 3.0.0,
  Rust/Cargo 1.99.1, qemu-user 11.1.3.  ALL were actually used.
* GCC torture corpus restored with `scripts/ensure_gcc_torture_git.py` at
  commit `2397b3b0422d72744b03d1330dc346db4b81846f` (2016 compile sources).
  Per the "no local CI" instruction, the 40,100-pair differential campaign
  was NOT run locally; hosted CI is authoritative.
* `readelf`, `nm`, `objdump`, `qemu-*`, Python `struct` ELF header parsing,
  and `RUSTC_BOOTSTRAP=1 -Zprint-type-sizes`-equivalent unit assertions
  (`size_of`/`align_of`) were used.  `objtool`, `pahole`, `valgrind`, `perf`
  are absent from the sandbox.
* ICX 2026.2 is NOT available locally: no `icx`/`icpx` in the cache, the
  Intel PPA is outside the sandbox egress allowlist, and the only
  `intel/llvm` release asset 302-redirects to
  `release-assets.githubusercontent.com` (blocked).  NO local number is
  attributed to ICX.  Hosted CI's authenticated `icx-oracle` job is the
  third, separate oracle.  See `engineering/ICX-LOCAL-FEASIBILITY.md`.
* GDB 18.1 was restored and used for the CLI pipe-panic proof in the previous
  PR; this PR's one-build-per-function claim is additionally pinned by
  `structural_failures` across all four prologues and by check 6.  No new
  wall-clock claim is made for the i686 hoist (a `break getenv` cannot be set
  in this glibc).

## 6. Follow-up list — status

| item | status |
| --- | --- |
| O(n^2) duplicate-chain re-walk | FIXED + witness + 2 adversarial tests |
| Failed aligned probe re-walks whole prefix | FIXED (resume) + exact bound test |
| No EXECUTED cross-ISA evidence | FIXED: standalone 4-ISA harness (28 cases) + hosted lane fixtures (16 cases) |
| Structural ratchet for all of the above | FIXED: check 13, 43 non-vacuous rejection paths |
| Compiled layout / ZST proof | FIXED: `index_layout_has_no_padding_and_the_counter_is_a_zst` |
| i686 per-parameter process-global read | FIXED: one read/function, explicit bool |
| Strict Clippy `-D warnings` | FIXED: RC=0 |
| Benchmark measured the parser, not the index | FIXED: flat statement bodies, documented |
| General "beats GCC/Clang/ICX" claim | NOT claimed; explicitly rejected |
| ICX 2026.2 locally | NOT AVAILABLE; hosted `icx-oracle` only |
| 40,100-pair A/B corpus | DEFERRED to hosted CI per instruction |

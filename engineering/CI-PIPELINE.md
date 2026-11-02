# CI pipeline shape

This document is the map of the hosted pipeline: which lane runs what, which
cost each decision was bought with, and which gate enforces each invariant.
It exists because the pipeline is now *parallel by construction* rather than a
single dependency wave, and a reader must be able to tell a load-bearing
dependency from an accident.

Baseline for every number below: run **39686387347** (2026-10-21,
`main` @ `e623708`), the last fully green run before this restructure —
**26 m 33 s wall clock, 11 jobs**. Per-job durations in that run:

| lane | job time | of which provisioning | of which the slowest gate |
| --- | --- | --- | --- |
| Build once and unit tests | 542 s | 205 s | 161 s (`cargo test`) |
| Assembler and linker contracts | 1042 s | 141 s | 703 s (fast contracts) |
| Codegen and runtime contracts | 788 s | 126 s | 338 s (benchmark outputs) |
| Complete benchmark corpus | 717 s | 125 s | 573 s (corpus) |
| Assertion-enabled units and corpus | 608 s | 153 s | 245 s (`cargo test`) |
| Frontend and ABI contracts | 484 s | 80 s | — |
| SSA regression corpus | 289 s | 107 s | 137 s (corpus) |
| Clippy / MSRV / ICX / Test Suite | 70 / 38 / 31 / 4 s | 0 s | — |

## The three costs that were paid, and what replaced them

### 1. The same apt transaction, seven times (`937 s` of runner time)

Six lanes plus the benchmark lane each ran
`scripts/ci_install_gcc16.sh --multilib` (80–205 s) and then
`apt-get install mold lld` (10–80 s) before doing any work. The package set is
immutable for a given (runner image, installer) pair, so it is now exported
**once** into a content-addressed tarball:

* `scripts/ci_toolchain_bundle.sh export|restore|verify` — tar of exactly the
  files owned by the package delta (`dpkg -L` of the `comm -13` of a
  `dpkg-query -W` ledger taken around the install), a `sha256` manifest, and a
  probe contract (`gcc-16 -m32` link, `mold`, `ld.lld`, `qemu-i386`).
* `scripts/ci_provision_toolchain.sh` — restore if the cache is hot, otherwise
  install (`ci_install_gcc16.sh` + **one** apt transaction for
  `qemu-user mold lld python3-yaml`), export, verify.
* `actions/cache` key:
  `lccc-toolchain-v1-${runner.os}-${hashFiles(installers)}`. A restore into `/`
  is escalated; a restore into a prefix is not (that is what keeps the contract
  test sudo-free).

`python3-yaml` rides in the same transaction, so the six `Install gate parsers`
steps are gone. Every lane provisions the **same** package set: a 64-bit-only
bundle cannot satisfy the shared `verify` contract (`gcc-16 -m32`), so a second
mode would be rejected and re-installed forever.

A cold cache still pays one install per lane (exactly today's cost); the
second run onwards pays a restore.

### 2. One 703 s step that was 140 independent gates

`Verify remaining fast local contracts` ran 140 gate invocations in sequence,
so the lane's wall clock was their sum. It is now **two steps in two jobs**
(`backend-contracts-a`, `backend-contracts-b`), assigned by longest-processing-
time (LPT) over per-gate measured cost, and each shard keeps the original
order and the original comments.

The split is a **partition**, and `tests/regression/check_ci_shard_partition.py`
enforces it from the workflow file:

1. the two shards' invocations are disjoint (nothing is paid twice),
2. neither shard is empty (a typo cannot become "0 gates ran, green"),
3. a shard invocation does not also run in another step (no cross-lane
   duplicate — this is what caught `check_llvm_scalar_logic.py` being run both
   in `frontend` and inside the fast-contract block),
4. every shard program is a registered **fast** gate in `scripts/ci_local.sh`
   (the local mirror is the register of record; `check_ci_gate_parity.py`
   closes the other direction, so together they prove the union is complete).

Re-balancing: measure per-gate cost (extract each logical command from the
step body, time it, and solve the 2-way LPT), then move whole gate blocks —
comments included — from one shard to the other. The contract above is what
makes that edit safe; the two shard budgets are stated in the comment above
each step.

### 3. The artifact existed 161 s before any consumer could use it

`build` compiled (133 s), then ran the whole test suite (161 s), and only then
did five lanes start. The suite moved to its own `unit` job, so the artifact is
published at ~165 s instead of ~540 s. The cost is one extra compile
(~135 s of runner time); the gain is ~160 s of wall clock for every dependent
lane, including the one on the critical path.

## What did NOT change, and why

* **The complete benchmark corpus stays one serial lane on one runner.** It is
  the timing evidence of record: `--reps 9 --warmup 1` over 49 benchmarks with
  paired, randomised compiler order. Sharding it across runners would make one
  report claim one machine while its numbers came from two. That is a
  provenance change, not a speed-up, so it is out of scope for a pipeline
  restructure. Consequence, stated plainly: the benchmark corpus is now the
  critical path (see below).
* **`cargo test` runs twice** on purpose — once with the release-profile flags
  and once with `debug-assertions=true`. Those are two different compilers;
  collapsing them would delete coverage.
* **The two benchmark gates are not duplicates.**
  `scripts/check_benchmark_outputs.sh` compiles 69 cases at four optimisation
  levels and compares stdout against a **checked-in reference**;
  `ci-bench.py --strict` compares 49 registry cases at `-O2` against a **live
  GCC build** while timing them. Different case sets, different oracles,
  different levels. The overlap is `-O2` on the intersection, and the honest
  fix is to run the first one four-wide (it compares bytes, never timings),
  not to delete it.
* **`CARGO_BUILD_JOBS: 2`** stays. It is a deliberate memory bound, and the
  build is not the critical path.

## Invariants and their owners

| invariant | enforced by |
| --- | --- |
| every registered gate runs in hosted CI, and only registered gates run | `scripts/check_ci_gate_parity.py` |
| the two contract shards are a partition | `tests/regression/check_ci_shard_partition.py` |
| a bundle restores only after its digest and its tools are verified | `tests/regression/check_ci_toolchain_bundle.py` |
| a `run:` body is shell, not YAML | `scripts/check_ci_workflow_shell.py` |
| the compiler each lane tests is the compiler `build` produced | `target/ci-build.commit` + `sha256sum -c` in every consumer |

## Where the wall clock goes now

Expected shape of a warm-cache run (provisioning restored from the bundle):

```
build ........................ ~165 s  -> the wave every consumer waits on
  unit ....................... ~225 s
  debug ...................... ~430 s
  corpus ..................... ~170 s
  frontend ................... ~290 s
  codegen .................... ~260 s   (output gate now four-wide)
  backend .................... ~110 s
  backend-contracts-a/b ...... ~400 s   (half of the old 703 s each)
  benchmark .................. ~600 s   <- critical path
```

The remaining lever is the benchmark corpus (~573 s of measurement). Splitting
it requires a merge step that preserves per-shard provenance and re-renders the
markdown summary from the union, plus a contract test that the union equals the
registry. That is a change to the evidence format, so it belongs in its own PR
with its own review, not in a pipeline restructure.

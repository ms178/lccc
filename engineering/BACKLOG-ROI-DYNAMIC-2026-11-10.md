# High-ROI backlog sweep — 16 executable items (2026-11-10)

Base: `470a775acdda2cb622ebe1873e2706b52dee2cb3` (`origin/main`, post-merge #1171),
candidate branch `arena/dd303e64-lccc` at the zero-copy SLP follow-up commit.
Method: [`engineering/agent/RULES.md`](agent/RULES.md). Executable half:
[`scripts/check_backlog_roi_dynamic.py`](../scripts/check_backlog_roi_dynamic.py).
Portable CI contract: [`tests/corpus/test_backlog_roi_dynamic.py`](../tests/corpus/test_backlog_roi_dynamic.py).
Machine-readable live evidence: `/tmp/roi_dynamic_evidence.json` (regenerate with
`python3 scripts/check_backlog_roi_dynamic.py --self-test --json --out FILE`).

## Why this sweep exists

`./backlog.md` is a 3,312-line programme index with explicit ROI numbers, but
prose statuses go stale. The 2026-11-05 sweep pinned 20 *static* ratchets in
`scripts/check_backlog_roi.py`; this sweep adds the complementary **dynamic**
layer. It does not infer an option's effect from a table row: it compiles an
option-on fixture and an option-off control, links ELF images, reads program
headers, compares flat-image bytes, runs macro/attribute diagnostics against
GCC 16.2, and executes the existing source-level peephole gate. Each of the 16
checks returns named problems and has a mutation in `--self-test` that it must
reject. Ten of the 16 items have root-backlog ROI ≥ 4.0, exceeding the requested
ten High-ROI items; the remaining six are lower-ROI supporting ratchets.

Selection rule: **ROI = measured contract × cost of the regression it prevents
÷ implementation risk.** Items whose full feature is still open (default PIE,
stack-clash probes, stack-protector canaries, a general loop vectoriser, AVX-512
opmask allocation) are *not* relabelled as closed. MB-04/3 pins only the explicit
`-pie` and `--build-id=sha1` effects that are shippable today; the default-PIE
gap remains recorded in `backlog.md`.

## Toolchain and exact live verdict

Pinned offline cache `harness-cache-8`: Rust 1.99.1, GCC 16.2.0, Clang 23.1.5,
GDB 18.1, binutils/readelf/objcopy 2.47.20260726, mold 3.0.0, QEMU 11.1.3,
Valgrind 3.19.0. Swap: exactly 4 GiB active. Fastbuild: `lccc`, `lccc-i686`,
`lccc-arm`, `lccc-riscv`, `lccc-x86`, `lccc-ld`. Local ICX 2026.2 is absent; the
hosted authenticated ICX lane is the ICX evidence. No wall-clock, overall
compiler victory, maximal per-function efficiency, or zero-issues claim is made.

Live command on this head:

```text
python3 scripts/check_backlog_roi_dynamic.py --self-test --json
  items=16  passed=16  failed=[]  self_test=[]
python3 scripts/check_backlog_roi_dynamic.py --offline --self-test --json
  items=16  passed=16  failed=[]  self_test=[]
```

## The 16 measured items

| # | ID | ROI | Root source | Measured live result | Negative control |
|---:|---|---:|---|---|---|
| 1 | MB-20/3 | 4.8 | §4 rank 3 | Source scanner PASS over 373 backend files; its own unittest module PASS; zero allocation before a rejecting filter. | Scanner rc mutated to 1. |
| 2 | MB-01/1 | 4.6 | §4 rank 1 | `-fPIC -O2 -fno-plt -S` emits 1 `GOTPCREL`; option-off control emits 0. | Flag-on `GOTPCREL` count mutated to 0. |
| 3 | MB-01/2 | 4.6 | §4 rank 1 | `-fvisibility=hidden` emits 1 `.hidden`; option-off control emits 0. | Flag-on `.hidden` count mutated to 0. |
| 4 | MB-02/1 | 4.5 | §4 rank 2 | `readelf -lW`: `-z execstack` → `GNU_STACK RWE`, `-z noexecstack` → `RW`. | RWE mutated to RW. |
| 5 | MB-02/2 | 4.5 | §4 rank 2 | Driver `-Wl,--oformat=binary` writes a flat image whose first bytes are not ELF and whose bytes exactly equal `objcopy -O binary` of the same link. | Byte equality mutated to false. |
| 6 | GP-22/5 | 4.5 | §12.1 rank 1 | `option_effect_ledger.py --lccc` executes 20 rows / 14 probes / 7 priority options and PASSes on all four drivers: x86-64, i686, AArch64, RISC-V. | AArch64 ledger rc mutated to 1. |
| 7 | GP-01/1 | 4.6 | §12.1 rank 2 | `__attribute__((frobnicate))` warns, names `frobnicate` and `-Wattributes`, and compiles rc0. | Warning-group flag mutated to false. |
| 8 | GP-23/1 | 4.0 | §12.1 rank 5 | `check_macro_parity.py` PASS: GCC 408 / LCCC 423 macros; budgets 13 GCC-only, 28 LCCC-only, 28 value diffs. | Gate rc mutated to 1. |
| 9 | GP-05/1 | 4.4 | §12.1 rank 3 | LCCC and pinned GCC 16.2.0 both define `__GNUC__ 16`; Clang 23.1.5 banner recorded; ICX remains a hosted-lane receipt. | LCCC major mutated to 14. |
| 10 | MB-12/1 | 3.4 | §4 rank 10 | `-Wformat` emits the typed diagnostic and rc0; `-Werror=format` emits `error:` and rc1. | `-Werror` rc mutated to 0. |
| 11 | GP-07/1 | 3.4 | §12.1 rank 17 | C digraph fixture `<% ... %>` compiles with rc0 and empty stderr. | rc mutated to 1. |
| 12 | GP-08/1 | 3.0 | §12.1 rank 22 | Default GNU C23 rejects `no_args(1)` after `no_args()`; explicit `-std=gnu17` accepts it. | Default rc mutated to 0. |
| 13 | GP-21/1 | 3.2 | §12.1 rank 20 | `-dumpfullversion` = `16.2.0`; five query flags rc0; `-print-multi-lib` names `32;@m32`. | Full version mutated to `15.2.0`. |
| 14 | MB-13/1 | 3.6 | §4 rank 8 | `lccc-i686 -c` emits `ELF32`, machine `Intel 80386`, with no 32-bit libc dependency. | ELF class mutated to ELF64. |
| 15 | SV-08/1 | unranked | §9b.3 SV-08 (effort S) | A 4-`ymm` fixture emits 1 `vzeroupper`; the positive control prevents a vacuous pass. | `vzeroupper` count mutated to 0. |
| 16 | MB-04/3 | 4.2 | §4 rank 4 | Explicit `-pie -Wl,--build-id=sha1` emits `ET_DYN` and a GNU Build ID note. Default PIE remains an open backlog item. | Type mutated to EXEC. |

## What is **not** claimed

- **Default PIE (MB-04 step 1) is still open.** The live default link is
  `ET_EXEC` with no build-id note; the dynamic check pins only the explicit
  contract so a future default-PIE change has a known-good control.
- **MB-05 / MB-06 hardening, SV-06 general vectorisation and SV-09 AVX-512
  opmasks remain open.** Honest refusals are better than plausible-looking but
  wrong probes; this sweep does not silently mark them implemented.
- **SV-07 `-O3` remains a policy question.** On the measured reduction fixture
  `-O2` and `-O3` still produce the same 487-byte assembly; the scoreboard
  assertion requested by the backlog is not yet delivered.
- **Instruction counts are not runtime wins.** No PMU, wall-clock, LTO, large
  model, full-ISA or 48-seed YARPGen result is claimed here.
- **ICX 2026.2 is not installed locally.** GCC 16.2.0, Clang 23.1.5 and LCCC
  are the local dynamic oracles; hosted CI's authenticated ICX lane is the
  third-compiler receipt.

## Hosted-CI red-team repair (2026-11-10)

The first hosted run of PR #1174 was red in `Assembler and linker contracts`,
`Fast local contracts (shard b)` and the aggregate `Test Suite`. The job logs
named one root cause in both independent lanes:

```text
FAIL: audit census grew:
  ignored_tests: 10 -> 11 (+1)
python3 scripts/audit_census.py --check (exit 1)
```

The extra ignored test was the Rust `#[ignore]` Callgrind microbenchmark in
the SLP patchset. An ignored test is not a regression gate and it violates this
repository's shrinking census. It was replaced by an always-on 64-repetition
stress test that asserts the 37-bucket partition is lossless and borrows the
original owned stream; Callgrind can still instrument that named test with
`--exact`. `build_plan` additionally rejects out-of-bounds store positions,
including `usize::MAX`, before any instruction indexing, and the malformed-seed
Rust matrix exercises both cases.

The harness-wipe snapshot helper was itself defective after commits landed: it
diffed only against `HEAD`, producing an empty patch from a clean tree. It now
diffs from `merge-base HEAD origin/main` through the working tree (with a
`HEAD^` fallback for shallow CI), marks new deliverables intent-to-add,
reverse-applies the full patch, and records the base selection in its JSON
manifest. A compiler-free corpus module pins both repairs and runs
`audit_census.py --check` in the fast hosted lane.

## Second hosted-CI red-team repair: shallow snapshot contract (2026-11-10)

The repaired head `a68deb891` turned 27 hosted checks green; only
`Fast local contracts (shard b)` (check-run `129936348147`) and the dependent
`Test Suite` aggregator remained red. The blob log named exactly one failing
unittest:

```text
FAIL: test_snapshot_script_produces_a_reverse_appliable_full_delta
AssertionError: 1 != 0 : error: No valid patches in input (allow with "--allow-empty")
save_ms178_1_snapshot.sh: reverse-apply check failed
::error title=Fast contract failed::python3 -m unittest discover -s tests/corpus -p 'test_*.py' (exit 1)
```

The cause was a test-design defect, not compiler codegen. GitHub's
`actions/checkout@v7` uses `fetch-depth: 1`; on that tree
`git merge-base HEAD origin/main` and `HEAD^` are both unavailable, so the
helper's root-commit fallback diffed `HEAD` against a clean work tree and
produced a zero-byte patch. `git apply --check -R` correctly refused it. The
earlier local test had passed only because the developer tree was deep and
dirty. The repair is threefold:

1. `MS178_1_ROOT` redirects every Git operation to an isolated work tree and
   is rejected unless `git rev-parse --is-inside-work-tree` succeeds. This lets
   the corpus contract construct the exact shallow shape without touching the
   real checkout or depending on GitHub network state.
2. The helper explicitly refuses an empty snapshot before publication with a
   named `empty snapshot refused` / `reverse-apply check failed` diagnostic.
   A silent zero-byte patch can no longer masquerade as a deliverable.
3. `tests/corpus/test_slp_patchset_hardening.py` now creates root-commit-only
   Git fixtures, publishes a non-empty reverse-appliable delta from an
   intent-to-add deliverable, applies that patch forward to a second clean
   tree, exercises the `MS178_1_NO_INTENT=1` tracked-edit path, and includes
   negative controls for the hosted empty-patch shape and a non-Git root.
   These are compiler-free, deterministic on shallow CI, and cannot pass on a
   vacuous patch.

This is a stricter contract than the failing one: it proves both directions of
the patch, both intent modes, the root override validation, and fail-closed
behavior on the exact hosted failure mode.

## Negative-control doctrine

A check that cannot fail is decoration. `negative_controls()` deep-copies the
observation set, mutates exactly one field per item, and requires the named
check to emit at least one problem. The corpus test also executes
`--offline --self-test --json`, which needs no compiler and therefore runs in
the hosted fast corpus lane. The live `--self-test` run above exercises the
same verdict machinery against real compiler observations. Both runs reported
`self_test=[]`.

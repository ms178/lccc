# FOLLOWUP-2026-10-18 — vellvm / vir red-team audit and transplant session

**Session base (upstream):** `ms178/lccc` `main` at session start = `d5b3d481baa9abc7b0054106897860fd45b2654e` (PR #855).
**Rebased at session end onto** `main` = `50167bc26ae4b43643ed44a4d63e10f00d83434e` (PR #857, 7 commits, all ancestors of the new head; none of the six touched files changed upstream). The patch is base-relative to `50167bc2`.
**Primary deliverable:** [`docs/VELLVM_VIR_TRANSPLANT_AUDIT.md`](../docs/VELLVM_VIR_TRANSPLANT_AUDIT.md)
— the audit, the transplant register T1–T14, the evidence, and the rejected ideas.
This file records what was done this session, how it was validated, and what the next
agent must do. It is the per-session follow-up requested by the owner; the compressed
journal digest is in [`journal/2026-10.md`](journal/2026-10.md).

---

## 1. Accomplished (validated)

| ID | Change | File(s) | Validation | Mutation / negative control |
|---|---|---|---|---|
| T1 | Code-disjoint set-equation dominator oracle + 2 tests (textbook self-check, 1 500 random CFGs vs SNCA and CHK) | `src/ir/analysis.rs` | `cargo test --profile fastbuild --lib dominator_tests` → 5/5 | `max_by_key`→`min_by_key`: both new tests FAIL; old CHK test still passes (proves the gap the oracle closes) |
| T2 | Self-use SSA regression `v9 = copy v9` must be "does not dominate its use" | `src/passes/verify/tests.rs` | `cargo test --lib passes::verify` → 32/32 | `di < ui`→`di <= ui`: new test FAILS |
| T3 | Byte-precise type-punning differential fuzzer (union / memcpy / byte access) with independent Python byte model | `tests/fuzz/punning_fuzz.py`, `tests/fuzz/test_punning_fuzz.py` | see §2 | model bugs found and fixed by the GCC self-check (§2.1) |

Formatting: `rustfmt --edition 2024 --check` clean on both touched Rust files; `cargo fmt --all -- --check` exit 0.

### 1.1 Documentation and bookkeeping

- `docs/VELLVM_VIR_TRANSPLANT_AUDIT.md` (new): audit + register.
- `engineering/FOLLOWUP-2026-10-18-vellvm-vir-transplant.md` (this file).
- `engineering/journal/2026-10.md`: dated digest section appended.
- `tests/fuzz/README.md`: section for `punning_fuzz.py`.
- `docs/licensing.md`: provenance row (concept-only use of MIT/Apache-2.0 sources; no code copied).

---

## 2. Measured results

### 2.1 Punning oracle (T3)

| Step | Command | Result |
|---|---|---|
| Model vs GCC (oracle self-check) | `punning_fuzz.py --ccc gcc --seeds 0:300 --levels O0,O1,O2,O3,Os` | **1 500 / 1 500 pass** |
| Generator definedness | 60 seeds, `gcc -Wall -Wextra -fsanitize=undefined,address -fno-sanitize-recover=all -O1` | **0 warnings, 0 sanitizer reports** |
| LCCC | `punning_fuzz.py --ccc target/fastbuild/lccc --seeds 0:1000 --levels O0,O1,O2,O3,Os` | **5 000 / 5 000 pass** (binary built from base commit; this session changed test code only) |
| Unit self-tests | `python3 -m unittest discover -s tests/fuzz -p 'test_*.py'` | 11/11 OK (7 are new) |

Two defects in the *first* model draft were caught by step 1, not by LCCC: the model
advanced the value stream on reads (750/750 mismatches) and parallel jobs for one seed
deleted each other's source (35 compile errors). **Lesson:** the oracle is validated
against a trusted compiler before any verdict about the compiler under test is drawn.

### 2.2 Dominator oracle (T1)

Oracle validated on the textbook shapes first (`set_equation_oracle_matches_textbook_shapes`),
then used as ground truth. Both production algorithms agree with it on 1 500 random CFGs.

### 2.3 `vir` red-team reproduction (for the audit)

| Step | Result |
|---|---|
| `lake build` (lean 4.27.0 via elan) | exit 0, 52 jobs |
| `lake test` at baseline | exit 0 |
| Mutant `InstrDomTree.lean:89` `<` → `<=`, **clean rebuild**, self-use scratch file | `operandsDominateUses = false` is **false** → self-use accepted (exit 1) |
| Same mutant, full `lake test` | **exit 0** → suite blind to the defect |
| Restored source + `lake build` + `lake test` | exit 0; `git status` clean |

Reproduction recipe (scratch file must be removed afterwards):

```bash
export PATH=$HOME/.elan/bin:$PATH; cd vir
cat > Vir/ScratchSelfUse.lean <<'EOF'
import Vir
open Vir
namespace SelfUse
def val0 : ValId := ⟨0⟩
def val1 : ValId := ⟨1⟩
def val2 : ValId := ⟨2⟩
def val3 : ValId := ⟨3⟩
def c0 : Val := .const (.int (0#32))
def c1 : Val := .const (.int (1#32))
def selfUse : Function :=
  { paramTys := [], returnTy := Ty.i32
    blocks := #[some { name := none, instrs := #[val2, val3] }]
    vals := #[some c0, some c1,
      some (.instr (.bin { kind := .add, ty := Ty.i32, lhs := val2, rhs := val1 })),
      some (.instr (.ret { value := val2 }))] }
example : selfUse.operandsDominateUses (InstrDomTree.buildFromFunction selfUse) = false := by
  native_decide
end SelfUse
EOF
lake build && lake env lean Vir/ScratchSelfUse.lean; echo rc=$?; rm Vir/ScratchSelfUse.lean
```

**Harness hazard observed:** a clean `lake env lean` after a source restore *without*
`lake build` silently used the mutant `.olean`s. Always `lake build` after a restore.

### 2.4 Source facts used in the audit

`vir`: 3 648 lines of Lean in `Vir/`; 8 `theorem`/`lemma`; 45 `native_decide`; IR = 5
instruction kinds. `vellvm`: 84 Rocq files / 45 958 lines; 1 041 `Lemma`/`Theorem`;
1 052 `Qed.`; 0 `Admitted.`; assumptions `ParserHelper.v:63`, `Archi.v:31`,
`Integers.v:60-61`. Full table in the audit §1.

---

## 3. Infrastructure state (this host, end of session)

**Host re-provisioning.** Late in the session the root filesystem was re-provisioned. Only `/home/user` persisted, and the system layer did not. **Lost and NOT reinstalled:** GCC 16 (sid), Clang 23.1.3, valgrind, mold, lld, qemu-user-binfmt (and the manual `qemu-i386` binfmt registration), and elan/Lean. **Rebuilt:** swap (8 G), rustup stable 1.99.1, and the fastbuild binary. The gate results in §6 come from the host **before** that reprovisioning. The rebased tree was validated afterwards with fastbuild plus targeted tests only (see §6).

| Item | State | Note |
|---|---|---|
| Swap | `/swapfile` 8 G via `scripts/ensure_swap.sh` (`LCCC_SWAP_SIZE=8G`) | Re-run after a harness wipe. |
| Rust | rustup stable 1.99.1 (clippy 0.1.99) + **1.97.0** (MSRV) | MSRV install was interrupted once by a full disk; reinstalled cleanly. |
| GCC oracle | **GCC 16.2.0-3 from Debian sid** (`gcc-16`, `g++-16`, `-multilib`, `libc6-dev-i386`) | Installed with `apt -t sid`, sid pinned at priority 100. `scripts/ci_install_gcc16.sh` only falls back to an Ubuntu PPA; on Debian it stops when `gcc-16` is not in the default repositories, so sid was added by hand. |
| libc | upgraded 2.41 → **2.43-7** (from sid, as a side effect of gcc-16) | valgrind had to come from sid as well to keep the libc6-dbg pair consistent. |
| Clang oracle | **Clang 23.1.3** via `scripts/ci_install_clang_oracle.sh` (SHA-256 pinned) | Needed `TMPDIR` on the real disk: `/tmp` is a 992 MB tmpfs and the 1.19 GB archive does not fit. |
| Linkers | mold **2.37.1**, lld **19.1.7** (Debian trixie) | The owner's target was mold 2.42.1 and LLVM 23 lld. Not met; not chased this session. |
| Valgrind | 3.27.1 (sid) | Used by the fma-contract gate. |
| qemu-user | `qemu-user` 10.0.13 + `qemu-user-binfmt`; `qemu-i386` registered manually in binfmt_misc | `systemd-binfmt` is masked on this host, so the handler was registered by hand. |
| Lean | elan, `leanprover/lean4:v4.27.0` under `~/.elan` (2.5 G) | Audit reproduction only. |
| Disk | 25 G root, about 4 G free at the end | Tight. `target/fastbuild/incremental` (3.5 G) was deleted; it is a cache. |
| Reference clones | `~/.cache/lccc-sources/vir` (pin c61e3517) and `…/vellvm` (pin 4c6db500) | Moved out of the workspace for the snapshot file budget. Their `.git` metadata was lost in the restore; content is present. |

**Workspace restore incident.** Late in the session the harness restored `/home/user`. The restore dropped `lccc/.git`, cleared the execute bit on 472 tracked files, and removed 14 tracked paths (`.github/`, `.cargo/config.toml`, two `tests/corpus/clang-c/` files). The repository was re-cloned from `ms178/lccc`, `git reset --hard` brought the worktree to the new `main`, and the ten session files were copied back from a backup, then compared again. The validation below was done before the restore, on the base `d5b3d481` tree; the rebase is a no-op for the six modified files (no upstream change).

**i386 execution on this host is broken for PIE binaries.** Non-PIE `gcc-16 -m32 -no-pie`
programs run (native and under qemu). Every PIE i386 program crashes with SIGSEGV, native
and under `qemu-i386`. `qemu-i386 -strace` shows an unsupported syscall **386** (`rseq`)
right before the crash. `GLIBC_TUNABLES=glibc.pthread.rseq=0` did **not** fix it.
The root cause is not established. Hypothesis: glibc 2.43 PIE start-up against
qemu-user 10.0.13. This is an environment limit, not an LCCC finding, and no gate was
weakened to avoid it.

## 4. Gate status and snapshot

- `scripts/ci_local.sh --fast`: see §6 (stamp + ledger are authoritative).
- Snapshot: the repository's own `scripts/lccc-snapshot.sh` is used. It is the evolved
  successor of the snapshot script given in the owner's brief: it adds the
  `ci_local.sh` pass-stamp gate, a 64 MiB / 256-file publisher cap, and a verified
  compressed patch series. It keeps the owner's invariants (atomic writes, patch
  base..HEAD as deliverable, ledger, source archive, bundle).
- Deliverable: `ms178-1.patch` at the workspace root (base `50167bc2`).

---

## 5. Open items (to-do for the next agent, in priority order)

Each item is also in the audit register (§5/§8 of the audit). Do not claim a bug for any
of them until a witness exists.

1. **T4 — C-level test-case reducer (HIGH).** LCCC has no miscompile reducer for C
   (`vellvm/utilities/vellvm-reduce.sh` is bugpoint-based and IR-only). Design: a
   delta-debugging shrinker over C text (line/token passes first, then AST-aware if
   the parser allows) with a two-oracle interestingness test: (a) the failure
   reproduces, and (b) the reference (`gcc -O0`) output is still defined. Mutation-test
   it on a deliberately broken pass.
2. **T5 — observable round caps (HIGH/MED).** Round loops in `loop_carried_forward.rs`
   (3), `backedge_pre.rs` (3), `reassoc_latency.rs` (8), `bool_thread.rs` (512),
   `loop_preheader.rs` (blocks+1), `inline.rs` (`max_rounds`) stop silently at the cap.
   Add a stats counter per pass that records "stopped at cap with work remaining", report
   it under the existing `-ftime-report`-style diagnostics, then measure how often it
   fires on the benchmark corpus before deciding anything.
3. **T7 — widen the punning oracle (MED).** Add `float`/`double` bit punning (union and
   memcpy), `__int128`, bit-fields, struct members with padding, and overlapping
   `memcpy`. Keep the GCC self-check first; the model must match GCC before LCCC is
   consulted. Add an `-O2 -march=x86-64-v3` lane.
4. **T9 — aggregate with undef/poison (MED, evidence needed).** Write C witnesses for
   partially initialised structs and unions that are read after an SROA-style rewrite
   (C analogue of Vellvm's known `insertvalue`-into-`undef` failure). Compare against
   GCC `-O0/-O2` and Clang. Only if LCCC disagrees is it a bug.
5. **T6 — IR verifier by default in more gates (MED).** Measure the compile-time cost of
   `CCC_VERIFY_IR=1` on the regression corpus; if acceptable, add one fast gate.
6. **Self-use coverage in `vir` (LOW, external).** The defect in §2.3 is upstream;
   no action unless the repo is ever adopted. Record only.
7. **Vellvm build (LOW).** If the Rocq toolchain becomes available, build
   `src/rocq` (`make -C src` per its README) and re-run the four axiom sites, to confirm
   none is load-bearing for the claims the audit repeated.
8. **Snapshot cleanliness (INFRA).** The repository's `lccc-snapshot.sh` gates on
   `target/ci_local.pass` for the *same* tree. Any edit after a gate run invalidates it:
   finish all edits, run the gate once, then snapshot.

### T-INF items

- **T-INF-1:** `scripts/build_lccc_fast.sh` prints "clang/mold not found". The owner's
  requested mold X86/i686 preset is unavailable on this host; the repository's own
  `.cargo/config.toml` preference is respected when present. Install clang+mold (or
  conda mold) to exercise the preferred linker path.
- **T-INF-2:** `lccc-snapshot.sh` requires `ci_local.sh --fast` to have passed on the
  exact tree. Budget ~1 h of wall time for that run on 2 cores.
- **T-INF-3:** The owner's linker-oracle request ("update the linker oracle to honour my
  build and version preferences") was **not** addressed in this session; it is a separate
  work item and must not be claimed as done. See `scripts/godbolt.py` and
  `docs/GODBOLT_ORACLE.md` for the oracle surface.

---

## 6. Gate result (end of session)

**`scripts/ci_local.sh --fast` is NOT green on this host. No pass stamp was written, so the
snapshot gate will refuse to publish this tree.**

| Run | Result |
|---|---|
| Full `--fast`, run 2 (all oracles installed) | 299 passed, 17 failed, 6 skipped. The failures were 14 i386-execution gates, `msrv-check` (toolchain interrupted, since fixed), `fma-contract-gating` (valgrind missing, since fixed), and `clippy` (my own deletion of `target/fastbuild/incremental` during the run). |
| `--only fma-contract-gating` | PASS |
| `--only msrv-check` | PASS (rustc 1.97.0) |
| `--only clippy` | PASS |

Still failing, all because of the host i386 limit in §3 (`the host cannot run i386 executables`,
or an i386 PIE child dying with `signal 11`):

`memory-value-contracts`, `byte-accessor-inline`, `ivsr-integer-domains`,
`ivwiden-exposed-latch`, `fp-extract-homes`, `empty-aggregate-abi`,
`i386-small-struct-abi-debt`, `ivsr-signed-affine-step`, `ivsr-vecaddr-struct-walk`,
`strict-packed-dag`, `minmax-reduction`, `affine-loop-fold`, `unsigned-affine-loop`,
`linker-suite` (6 i386 cases).

The two i386 legs that the regression gates assert by design are the reason these fail
rather than skip. A clean green needs an i386-capable runner: the GitHub `ubuntu` runner
(which the repository's CI uses) or a host where PIE i386 binaries run. That run has not
been done here, and this file does not claim it.

**Rebased tree (`main` 50167bc2), validated after the reprovision:**

| Check | Result |
|---|---|
| `scripts/build_lccc_fast.sh` (fastbuild) | exit 0, 3 m 41 s |
| `cargo test --profile fastbuild --lib -- dominator_tests passes::verify` | 37 passed, 0 failed |
| `cargo fmt --all -- --check` | clean |
| `python3 -m unittest discover -s tests/fuzz -p 'test_*.py'` | 11 OK |
| `scripts/check_doc_links.py` | OK |
| `ci_local.sh --fast` (full) | **not re-run** on the rebased tree: the oracles were lost in the reprovision and the i386 blocker is unchanged. The last full run was on `d5b3d481`. |

Two log lines look alarming but are not gate failures: a `CORRECTNESS FAILURE` line in the
harness-contract test is followed by that gate's own PASS, and the `godbolt`/`icc` "not honored"
warning is informational.

## 7. Decisions and conflicts (for the owner)

- **Per-session follow-up vs journal rule.** `engineering/journal/README.md` says new
  sessions append to the period file and do not create per-session documents. The
  owner's standing order asks for a follow-up document each session. This session
  follows the owner's order (as the EDG session did) and also appends the digest to the
  journal. Owner to rule if the policy should change.
- **Snapshot script.** The owner's brief contains an earlier snapshot script; the
  repository's `scripts/lccc-snapshot.sh` is the evolved version and is used.
- **Rejected ideas** (do not re-derive): `undef ≡ poison`, porting `vir`'s Lean IR, and
  running Vellvm's interpreter as an oracle. Reasons in the audit §7.

---

## 8. Session 2 (second turn, same day): environment repair, i386 root cause, T5

Base unchanged: `main` = `50167bc26ae4b43643ed44a4d63e10f00d83434e` (checked at the start of the turn; no upstream change).

### 8.1 Toolchain and oracles restored (no source change)

| Item | How | Result |
|---|---|---|
| Rust 1.97.0 (cargo, rustc, clippy, rustfmt, rust-std) | `rust-cache-1.97.0` tag, SHA-256 checked against its `MANIFEST.sha256`, installed to `~/.cache/lccc-harness/rust/toolchain` | `cargo 1.97.0`, `clippy 0.1.97` |
| Clang 23.1.3 oracle | `scripts/ci_install_clang_oracle.sh --print-bin` with `TMPDIR` on the real disk (the script needs `zstd`, installed from apt) | SHA-256 verified (`14d2f701…`), `~/.cache/lccc/clang-oracle/23.1.3/bin/clang` |
| qemu-aarch64 / arm / riscv64 8.2.2 | `harness-cache-3`, SHA-256 checked, symlinked into `~/.local/bin` | run OK |
| Trixie `libc6-i386` 2.41 | `apt-get download libc6-i386=2.41-12+deb13u4`, extracted under the cache | used only for the i386 experiment in §8.2 |

Gates that were red for missing oracles, now green (`--only`, same tree): `fma-contract-gating` (needed Clang and Valgrind), `pr824-boundaries` (needed qemu-aarch64), `ext-vector-cross-abi`, `ext-vector-abi-grid` (needed Clang).

### 8.2 i386 PIE root cause (investigated, not fixed; environment)

Question: is the dynamic-PIE i386 crash a glibc, toolchain, or kernel issue?

| Experiment | Result |
|---|---|
| `int main(){return 0;}`, `gcc-16 -m32 -fPIE -pie` (Sid glibc 2.43) | SIGSEGV (rc 139) |
| Same binary, `gcc -m32 -fno-pie -no-pie` or `-fPIE -no-pie` | rc 0 |
| Same PIE with Trixie `ld-linux.so.2` + `libc.so.6` 2.41 (`--library-path`) | SIGSEGV (rc 139) -- not a glibc 2.43 regression |
| Static, static-PIE, libc-free PIE | rc 0 |
| vDSO hypothesis: call `AT_SYSINFO` with `getpid`, compare `%esp` | `%esp` delta 0 -- vDSO entry preserves the stack |
| gdb: breakpoint at `main`, inspect return slot | the return slot of `main` holds `argc` (1); `main` itself disassembles correctly |
| gdb hardware watchpoint on that slot from `_start` | never fires before the SIGSEGV; the value is not written by any user-mode store the debugger sees |

Conclusion: the failure reproduces with two glibc versions, natively and under qemu-i386 8.2.2, for a program that does nothing but return. It is specific to dynamically linked i386 PIE on this host's kernel (6.1.158+). No compiler or glibc change is implicated, and the vDSO path is ruled out. Root cause is **not** established beyond that. The gate exception already approved by the owner stands. The i386 gates were **not** weakened, and no PIE handling in the gates was changed.

Note on `gdb record`: `record full` fails on this host, so reverse-stepping output from an earlier attempt is not evidence and is not used here.

### 8.3 T5 — observable round caps (done as observability)

- New `src/passes/round_cap.rs`: a `Site` enum (5 sites), process-global relaxed atomic counters, `report()` printed under `CCC_TIME_PASSES` at both pipeline exits. No new environment reads (the pass-pipeline env ratchet forbids them).
- Sites converted: `loop_carried_forward`, `backedge_pre`, `reassoc_latency`, `bool_thread`, `loop_preheader`. A hit is recorded only when the loop exits without its convergence break.
- `inline.rs` is deliberately not converted: its budgets (`ordinary_rounds_left`, the second always-inline pass) are not the simple `for _ in 0..N` shape, and converting it needs its own design (follow-up T5b).

**Measurement (2 654 C files under `tests/`, `-O2`, `CCC_TIME_PASSES=1`):**

| Site | Files where the cap bound |
|---|---|
| `backedge_pre` (cap 3) | 2 (`cross_pr_redteam.c`: 1 hit; `ra_folded_index_wave_follow.c`: 2 hits) |
| others | 0 |

**Cap-raise experiment (cap 3 → 12, temporary scratch build, reverted):**

| File | Asm lines (cap 3 / 12) | Output correct? | callgrind Ir (cap 3 / 12) |
|---|---|---|---|
| `cross_pr_redteam.c` | 2093 / 2089 | yes (`FAILS 0`, same as GCC 16 -O0) | 8 715 489 / 8 715 483 |
| `ra_folded_index_wave_follow.c` | 716 / 709 | yes (`OK`, same as GCC 16 -O0) | 166 718 / 166 705 |

Decision: **keep the cap at 3.** The extra rewrites are correct but move the measured dynamic instruction count by less than 0.01 %, which is below anything worth a compile-time cost on the whole corpus. This is a measurement on two self-checking tests, not a benchmark claim.

**Gate:** `tests/regression/check_round_cap_observability.sh` (`round-cap-observability` in `ci_local.sh --fast`). Static: each `Site` has exactly one `note_cap_hit` call, and no call names an undeclared site. Behavioral: `cross_pr_redteam.c` prints `round_cap backedge_pre` under `CCC_TIME_PASSES=1` and nothing without it.
Mutation check: removing the `BackedgePre` call makes the gate fail (`FAIL: Site::BackedgePre has 0 note_cap_hit call(s)`, rc 1).

### 8.4 Validation this session

| Check | Result |
|---|---|
| `cargo build --profile fastbuild --locked` with `RUSTFLAGS=-D warnings` | rc 0, no warnings |
| `cargo clippy --profile fastbuild --all-targets -- -D warnings` | rc 0, no warnings |
| `cargo fmt --all -- --check` | clean |
| `cargo test --profile fastbuild --lib round_cap` | 3 passed |
| `ci_local.sh --only round-cap-observability` | 1 passed |
| `ci_local.sh --fast` (before T5 edits, on `7fe2d1b`) | 300 passed, 18 failed, 6 skipped; the 18 are the i386 gates (§8.2) plus the 4 oracle gates fixed in §8.1 |

Final `ci_local --fast` on the final tree (T5 code, the new hosted-step line, and this document): **305 passed, 14 failed, 6 skipped**. The 14 failures are the i386 execution gates listed in §8.5 (exit 139 or the `host, not lccc` probe). `round-cap-observability`, `ci-gate-parity`, `fma-contract-gating`, `pr824-boundaries`, `ext-vector-cross-abi`, and `ext-vector-abi-grid` are green.

### 8.5 Open items after this session

1. **i386 gates (blocked, owner-approved exception).** About 14 gates need i386 PIE execution, which this host cannot do (§8.2). Clean green needs an i386-capable runner. Not claimed here.
2. **T4 C reducer, T7 punning extensions, T9 aggregate-undef witnesses, T6 IR verifier in gates:** unchanged from §5. Not started this session.
3. **T5b `inline.rs` budgets:** needs its own counter design.
4. **Hosted-step parity.** `.github/workflows/ci.yml` gained `check_round_cap_observability.sh` in the fast step, because `check_ci_gate_parity.py` requires every standalone `ci_local` gate to be mirrored there. Without it the parity gate fails.
5. **Snapshot gate.** `lccc-snapshot.sh` requires a `ci_local --fast` pass stamp on the exact tree. The stamp is written only when every gate passes, so with the i386 gates red it stays unwritten. The owner-approved UNGATED path applies (see `LCCC-DELIVERY.md`).

### 8.6 Failing i386 gates (exact list, final run)

`memory-value-contracts`, `byte-accessor-inline`, `ivsr-integer-domains`, `ivwiden-exposed-latch`, `fp-extract-homes`, `empty-aggregate-abi`, `i386-small-struct-abi-debt`, `ivsr-signed-affine-step`, `ivsr-vecaddr-struct-walk`, `strict-packed-dag`, `minmax-reduction`, `affine-loop-fold`, `unsigned-affine-loop`, `linker-suite`.

Each one either cannot build or run a `-m32` PIE program on this host (§8.2), or runs its i386 leg and dies with signal 11. None of them was changed to skip or pass.

## 9. Session 3 (second turn): red-team fixes, T4/T6/T7/T9 done, ICX 2026.2 installed

### 9.1 Done this session (validated)

| Item | Status | Evidence |
|---|---|---|
| Round-cap counters are per-instance (`CapCounters`); tests no longer race on process-global state | DONE | `src/passes/round_cap.rs`; `counters_are_exact_under_concurrent_notes` (4 threads x 1000 notes = 4000) |
| `report()` drains counters (take-and-reset), so each module reports only its own hits | DONE | `round_cap.rs` `CapCounters::take`; unit test `counters_are_per_site_and_take_resets` |
| T5b: `inline.rs` ordinary and second-pass round budgets count a hit only when the budget reaches 0 and the last round still inlined | DONE | `src/passes/inline.rs` (flags `ordinary_round_inlined`, `second_pass_round_inlined`); sites `inline_ordinary_rounds`, `inline_second_pass_rounds` |
| T5b measurement, 2 654 `tests/**/*.c` at `-O2`, `CCC_TIME_PASSES=1` | MEASURED | `inline_ordinary_rounds` binds in `fortify_open_va_pack.c` (2 hits) and `cpu_model_mul_const_plan.c` (1 hit). `inline_second_pass_rounds` never binds. Raw: `/home/user/.cache/lccc-ci/round_cap_v2.tsv` |
| T5b budget test: 200 -> 1000 ordinary rounds | DECIDED: keep 200 | Both affected files produce byte-identical assembly (`diff` 0 lines). The flag is therefore an **upper bound** on lost work: "last round inlined" does not prove the next round would inline. Documented in the code comment, not hidden. |
| Observability gate witnesses both new/old sites | DONE | `tests/regression/check_round_cap_observability.sh`: `backedge_pre` on `cross_pr_redteam.c`, `inline_ordinary_rounds` on `fortify_open_va_pack.c`; PASS |
| T6: `CCC_VERIFY_IR=abort` cost and cleanliness | MEASURED | `scripts/measure_verify_ir_cost.py`. Full sweep at `-O2`: 2 656 files, 1 998 compile without the verifier, total ratio **1.047** (median 1.024, p90 1.125, max 2.111), **0 violations**. Raw: `/home/user/.cache/lccc-ci/verify_cost_full_O2.json` |
| T6 gate | DONE | `tests/regression/check_verify_ir_clean.sh` over `verify_ir_gate_files.txt` (80 files, every 25th clean file of the sweep); ~5 s; PASS |
| T4: C-level miscompile reducer | DONE | `scripts/c_reduce.py` (ddmin + 1-minimal fixpoint, memoised, budget keeps accepted progress, predicate error aborts) and `scripts/c_reduce_predicate.py` (two-oracle: GCC `-O0` with UBSan+ASan must run clean and deterministic, then compare the compiler under test). Demo: 13-line miscompile reduced to 5 lines in 49 predicate calls, 1-minimal |
| T4 mutation tests | DONE | `tests/fuzz/test_c_reduce.py` (8 tests): fake miscompiler `tests/fuzz/fake_miscompiler.py` must be found and reduced; correct compiler must give exit 2; out-of-bounds candidate must be rejected by the reference oracle; predicate exit 2 must abort (exit 1) |
| T7: punning oracle widened | PARTIAL | Added `f32`/`f64` (bit patterns, exponent forced finite), `u128`/`i128` (16-byte members, two-word hash), and the `v3` lane (`-march=x86-64-v3`). **Not done:** bit-fields, struct padding, overlapping `memcpy` (stays open, see 9.3) |
| T7 model self-check, all three oracles | DONE | GCC, Clang 23.1.3 and ICX 2026.2: 240/240 pass each (60 seeds, `-O0,-O2`, `base,v3`) |
| T7 LCCC verdict | DONE | `target/fastbuild/lccc`: 3 000/3 000 pass (300 seeds, `O0,O1,O2,O3,Os`, `base,v3`) |
| T7 mutation check | DONE | Wrapper that corrupts float stores: 12 of 20 seeds report `mismatch`; the 8 passing seeds have no float stores to corrupt |
| T9: aggregates with partial initialisation read after SROA-style rewrite | DONE (no disagreement) | `tests/regression/aggregate_partial_init_sroa.c`: only written members are read (UB-free, sanitizer-checked). GCC, Clang 23.1.3, ICX 2026.2 and LCCC give the same stdout (sha1 `0f07456a40b5`) at `-O0`..`-O3`. Under the owner's "only if LCCC disagrees is it a bug" rule, no bug |
| Lint and format | CLEAN | `cargo fmt --check` PASS; `scripts/check_clippy_policy.py` PASS, 0 warnings; pyflakes clean on all new Python; `check_ci_gate_parity.py` PASS (290 commands) |

### 9.2 Environment

* Intel oneAPI apt repository added (`/etc/apt/sources.list.d/intel-oneapi.list`, key `/usr/share/keyrings/intel-oneapi.gpg`). The signature verified on `apt-get update`.
* ICX 2026.2.0 (build 2026.2.0.20261001) installed by extracting the 10 needed `.deb`s into `/home/user/.cache/icx-2026.2/root` (2.2 GB installed). Full `apt-get install` needed about 3.3 GB plus the download, and the disk has one drive with about 5.8 GB free. `source /home/user/.cache/icx-2026.2/env.sh` puts `icx` on `PATH`. Verified: `icx -O2 -c` compiles.
* Clang 23.1.3 oracle at `/home/user/.cache/lccc/clang-oracle/23.1.3/bin/clang` (install log `/home/user/.cache/lccc-ci/clang_install2.log`, `clang --version` reports 23.1.3).
* Disk after the ICX extract: about 3.1 GB free on `/`. Keep it in mind before snapshots and `ci_local` runs.

### 9.3 To do (open, in priority order)

1. **T7 remainder:** bit-fields (signed/unsigned, straddling units), struct members with padding (read only named members; padding stays indeterminate), overlapping `memcpy` as the defined form only where the standard allows it. Requires a model extension and a GCC self-check before any LCCC verdict.
2. **T9 depth:** the single witness file found no bug. A generated corpus of partially initialised aggregates (the punning generator's shape, with struct members instead of union arrays) would test more SROA shapes. Not started.
3. **T5b refinement:** an exact "another round would have inlined" probe needs the site-selection context to be recomputed at exit. Currently a flag over-approximates; it is safe to leave as an upper bound.
4. **Performance comparison:** this session's changes do not alter codegen (the inline budget change is byte-identical; the verifier and reducer are not in the codegen path). A benchmark comparison against GCC 16.2, Clang 23.1.3 and ICX 2026.2 on zlib-ng, gzip and expat is still open.
5. Carried over unchanged: vir self-use (LOW, record only); Vellvm build (LOW); mold preferred-linker wiring (INFRA); linker-oracle preference (not done); i386 red gates (see 8.6).

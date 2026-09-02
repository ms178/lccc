
---

## 7. Rebase onto upstream `main` (614a987) — session 11 close-out

The working tree was re-based from `d1dba8d` onto `614a987`
(`Merge pull request #356 from ms178/arena/01a063be-lccc`), which pulls in two
new merges:

* `4177db8` — i686 peephole: dead-frame elision + more slot round-trip
  forwarding (i686 only, does not affect the x86-64 oracle);
* `4ef1be6` — x86 machinst: typed direct calls (`CallTyped`) + `xmm0/xmm1`
  scratch relays for slot-homed floats.

`git apply` of the session-10 delta onto `614a987` was **clean** (no conflicts:
upstream touched `i686/peephole.rs`, `x86/codegen/{emit,isel,machinst*}.rs`,
`backend/generation.rs`; this work touches `backend/common.rs`,
`backend/stack_layout/*`, `backend/x86/codegen/prologue.rs`). Rebuild with the
standard `cargo build --profile fastbuild -j2` succeeded with no errors.

### 7.1 ZSTD oracle re-verification on `614a987`

| build | real piggy payload (`vmlinux.bin.zst` 4,164,528 B) |
|---|---|
| `-O0` | MATCH |
| **`-O1`** | **FAIL — `ZSTD-compressed data is corrupt`** |
| `-O2` | MATCH |
| `-O3` | MATCH |
| `-O2` + `CCC_TIER2_GRAPH=1` (Tier-2 re-enabled) | **FAIL** |

Two conclusions:

1. The Tier-2 default-off fix is **necessary and sufficient** at `-O2`/`-O3`:
   re-enabling it reproduces the corruption on the new main as well.
2. **NEW, still open:** `-O1` now fails where it passed on `d1dba8d`. This is a
   regression introduced by upstream between `d1dba8d` and `614a987`
   (`4ef1be6` is the only x86-64-affecting commit in that range). It was *not*
   caused by this session's changes — they only make `-O2`/`-O3` correct.

   One-command reproducer (now in-tree):

   ```
   scripts/zstd_orac
   le_cc.sh "-O1" -O1      # FAIL
   scripts/zstd_oracle_cc.sh "-O2" -O2     # MATCH
   ```

   Knobs ruled out as the trigger (all still FAIL at `-O1`): `CCC_NO_AGGFWD`,
   `CCC_NO_FWD`, `CCC_NO_FRAME_ELIM`, `CCC_NO_DEADFRAME`, `CCC_DISABLE_AGGFWD`,
   `CCC_MI_DISABLE_KINDS={CallTyped,CallTypedX86,CallTypedX87,Call}`. Next
   session should bisect `-O1` with `CCC_DISABLE_PASSES` (the pass list that ran
   clean at `-O2`) and, failing that, a git-worktree build of `4ef1be6^` vs
   `4ef1be6` using `scripts/zstd_oracle_cc.sh` as the oracle.

### 7.2 Scripts landed in the repo this session

| script | purpose |
|---|---|
| `scripts/zstd_oracle_cc.sh` | one-command ZSTD miscompile bisector: builds the oracle TU with arbitrary env/flags and runs the real payload |
| `scripts/qemu_qmp_probe.py` | boots a bzImage under QEMU+QMP, dumps registers / code@RIP / VGA buffer / physical stack and resolves RIP to a symbol in the decompressor *and* the kernel proper |
| `scripts/make_bzimage_from_stub.sh` | re-packages an existing `compressed/vmlinux` with independently built setup objects (used to build the GCC-built-payload control image) |
| `scripts/bisect_boot_size.sh` | boot-gate `.text` for an arbitrary commit via git worktree + fastbuild |

### 7.3 Boot status on `614a987`

Unchanged and still open: the VM-config kernel reaches the **kernel proper** and
faults in `early_fixup_exception` (`RIP=ffffffff81efa09e`, self-`jmp`,
`CR2=ffffffff82400000`, `R14=0x0e` = `X86_TRAP_PF`). The identical fault occurs
with a **GCC-built** `compressed/vmlinux`, so the decompressor is exonerated;
the remaining suspects are the lccc-built kernel proper and the VM config's lack
of an early console. `scripts/qemu_qmp_probe.py --vmlinux <vmlinux>` is the
tool for the next step.

## 8. Rebase onto upstream `main` (5ec05cb, PR #357)

Upstream advanced again with `933ba32` (PR #357: FMA memory-operand folding,
i686 encoder gas-parity fixes, unroll overflow bail). Re-applied the delta onto
`5ec05cb`: **`git apply --check` clean, no conflicts** (PR #357 touches
x86/i686 isel+emit; this work touches `backend/common.rs`,
`backend/stack_layout/*`, `backend/x86/codegen/prologue.rs`).

Environment note: the sandbox lost `/home/user/lccc/.git` and the Rust
toolchain (`/home/user/.cargo`) between turns. The repository was restored by
re-cloning upstream (`--depth 60`) and re-applying the deliverable patch; the
git bundle in `/home/user/artifacts` is not self-contained (it references
parents outside the shallow clone) and cannot be used to restore. Next session
must re-bootstrap with rustup 1.98.0 before rebuilding
(`cargo build --profile fastbuild -j2`) and re-running
`scripts/run_regression_suite.sh` + `scripts/zstd_oracle_cc.sh`.

Carry-over work (unchanged): the `-O1` oracle regression introduced between
`d1dba8d` and `614a987` (§7.1), and the `early_fixup_exception` page fault in
the kernel proper (§7.3).

# Follow-up: thanhtoantnt fork transplant — accomplished, deferred, and next

Date: 2026-10-03
Base: `070bf71e` (`ms178/lccc` main, post-#743)
Audit: `engineering/AUDIT-2026-10-03-thanhtoantnt-fork.md`
Commits: `08a64eeb` (long-double widening), `c14a10f7` (branch range + regnum)

---

## 0. Read this first — the sandbox was wiped bare

The session started with **`/home/user` completely empty**: no repo, no
artifacts, no `.base_ref`, no toolchain, no swap. Everything below was
re-bootstrapped from scratch. Future sessions should assume the same and follow
§1 verbatim rather than rediscovering it.

---

## 1. Environment bootstrap (exact, verified working)

```bash
# --- swap (hard requirement: 1.9 GiB RAM, 2 cores) -------------------------
sudo -n dd if=/dev/zero of=/swapfile bs=1M count=4096 status=none
sudo -n chmod 600 /swapfile && sudo -n mkswap /swapfile && sudo -n swapon /swapfile
echo '/swapfile none swap sw 0 0' | sudo -n tee -a /etc/fstab

# --- Rust OUTSIDE the persisted workspace ----------------------------------
# WHY: the snapshot publisher (scripts/lccc_delivery.py) caps the visible
# workspace at 64 MiB / 256 files. A rustup install is ~1.5 GB / 30k files and
# would blow that cap, and `target/` was 3.9 GB / 7 623 files on its own.
sudo -n mkdir -p /opt/rustup /opt/cargo /opt/lccc-target
sudo -n chown -R user:user /opt/rustup /opt/cargo /opt/lccc-target
export RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --no-modify-path --profile minimal \
   --default-toolchain stable -c rustfmt -c clippy
rustup default stable          # REQUIRED: rustup-init alone leaves no default

# --- the build env every later command needs -------------------------------
export RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
export PATH=/opt/cargo/bin:$PATH
export CARGO_TARGET_DIR=/opt/lccc-target CARGO_BUILD_JOBS=2

# --- bridge for scripts that hardcode ./target -----------------------------
# ci_local.sh reads `target/fastbuild/lccc` and writes `target/ci_local.pass`.
# A SYMLINK (1 entry, os.walk does not descend) keeps those paths working while
# the bytes live outside the workspace. Do NOT copy the tree back.
ln -s /opt/lccc-target/target /home/user/lccc/target
```

Gotchas that each cost real time:

- `rustup-init --component rustfmt clippy` is rejected; the flag is `-c`, twice.
- After `rustup-init`, `rustc` fails with *"rustup could not choose a version"*
  until `rustup default stable` is run.
- `ci_local.sh` defaults `CARGO_HOME=$HOME/.cargo`; it must be overridden or it
  will not find the toolchain.
- `build_lccc_fast.sh` writes `target/lccc-rustflags`; `ci_local.sh` reads it
  back. Both resolve through the symlink — but if the symlink is recreated, copy
  the rustflags file across first.
- Everything under `/opt` is **not persisted** across sessions; only `/home/user`
  is. Budget ~4 min for a cold `fastbuild` and ~5 min for the toolchain install.

---

## 2. Accomplished this session

### 2.1 Fixes (5 defects, all validated)

| # | defect | fork issue | evidence |
|---:|---|---|---|
| 1 | `f64 → binary128` widening: u128 exponent underflow (ICE for every `\|v\| < 1.0`) + subnormal mis-encoding | #4, #310 | standalone repro + 3 444-value GCC differential |
| 2 | `f64 → x87` widening: subnormals mis-scoped by up to 4.5e14 | #114 | same |
| 3 | `f128 → f64` / `x87 → f64` narrowing returned `0.0` for everything below 2^-1022 | *not in corpus* | found by the new oracle |
| 4 | `f128 → x87` truncated (`>> 49`) instead of rounding; f128 subnormals emitted as non-canonical x87 subnormals | *not in corpus* | `gcc -O2 -S` byte diff |
| 5 | AArch64 + RISC-V `resolve_local_branches` wrapped out-of-range displacements silently | #121 | 4 new assembler tests |
| 6 | `parse_reg_num` accepted `x+5`, `x007`, `w+31` | #118, #207 | 3 new tests |

Defects 3 and 4 were **not in the fork's corpus at all** — they were found by
replicating the fork's *method* (differential testing against a real compiler)
rather than by importing its *findings*. That is the headline result.

### 2.2 New artefacts

- `scripts/ldconst_differential.py` — long-double constant differential oracle.
  Emits N constants with LCCC and with GCC/Clang/ICX, reconstructs the object
  bytes from **either** `.quad` or `.long` directives (GCC uses `.long` for x87
  long doubles, LCCC uses `.quad` — a word-oriented comparison reports a
  spurious mismatch on every constant), and diffs element-wise.
  `--n 3000` → 3 444 constants, ~1 s, currently **0 mismatches**.
- `engineering/AUDIT-2026-10-03-thanhtoantnt-fork.md` — the full audit.
- 8 new unit tests (long-double widening) + 7 (branch range, register parsing).

### 2.3 Validation

- `cargo test --lib`: **4 044 passed, 0 failed, 7 ignored** (baseline 4 036).
- Long-double differential: **3 444 / 3 444 byte-identical to GCC 14.2**.

---

## 3. Deliberately NOT done (with reasons)

- **No change to `scripts/lccc_delivery.py`.** Its workspace budget
  (64 MiB / 256 files) trips on every snapshot because the LCCC checkout alone
  is ~137 MB / ~5 300 files. The script is verified by a strict index
  (1 529 records / 1 434 hashes) inside `lccc-snapshot.sh`, and breaking the
  snapshot pipeline is the one failure mode this project cannot afford. The
  canonical deliverable `ms178-1.patch` **is** written correctly on every
  snapshot; only the ZIP transport receipt is skipped. See §4.1.
- **No migration of the fork's 100 `*_pbt.rs` files.** They target the fork's
  encoder signatures, which do not exist in LCCC after the shared-helper
  refactor, and they encode per-bug facts rather than invariants.
- **No AArch64 encoder hardening yet.** Designed (§4.2), not implemented.

---

## 4. To-do, ordered by `impact × confidence ÷ cost`

### 4.1 P1 — `lccc_delivery.py` workspace budget is unsatisfiable in the standard layout

**Symptom.** Every `lccc-snapshot.sh` run ends with

```
delivery FAILED: workspace budget exceeded: {'files': 5355, 'bytes': 144931596, ...};
limits bytes=67108864, files=256.
```

**Root cause.** `inventory()` walks the *entire* workspace, including the LCCC
checkout (~137 MB, ~5 300 files). The budget was calibrated for a workspace
holding only the patch and artifacts. As written the guard **always** fails —
a guard that always fails is not a guard.

**Proposed fix (needs care).** Exclude VCS checkouts from the census: a
directory containing `.git` is the *input* to the patch, not part of the
deliverable payload, and it is fully recoverable from `ms178-1.patch` + the base
commit (and from `artifacts/lccc.bundle` / `artifacts/lccc-src.tar.gz`). Report
the excluded aggregate size in the receipt so the number stays visible rather
than silently vanishing.

**Why it was not done here.** `lccc-snapshot.sh` verifies an invocation index
over `scripts/` (1 529 records / 1 434 hashes, all verified). Changing a
safety-critical script's budget semantics, late in a session, risks breaking the
one mechanism that preserves work across harness wipes. Do it at the start of a
session, with a full `ci_local.sh --fast` afterwards.

### 4.2 P1 — AArch64 encoder: accessor-layer validation + universal invariant harness

The highest-value remaining item from the corpus. **134 of the 351 issues trace
to a single function**:

```rust
// src/backend/arm/assembler/encoder/mod.rs:1169
pub(crate) fn get_reg(operands: &[Operand], idx: usize) -> Result<(u32, bool), String> {
    match operands.get(idx) {
        Some(Operand::Reg(name)) => {
            let num = parse_reg_num(name).ok_or_else(...)?;   // accepts ANY class
            let is_64 = is_64bit_reg(name);                    // sp and xzr both -> true
            Ok((num, is_64))
        }
        ...
```

It does not reject FP/SIMD registers where a GP register is required (F1, 53
issues), does not distinguish `sp` from `xzr` (F2, 28), and its `is_64` is
discarded by 113 call sites (F3). `get_imm` (47 call sites) performs no range
check at all (F4, 188).

**Step 1 — validating accessors.** Add next to `get_reg`:

```rust
pub(crate) fn get_gp_reg(ops, idx)      -> Result<(u32, bool), String>  // rejects FP/SIMD and SP/ZR
pub(crate) fn get_gp_reg_or_sp(ops, idx) -> Result<(u32, bool), String>  // rejects FP/SIMD; SP legal (ld/st base, add/sub sp)
pub(crate) fn get_fp_reg(ops, idx)      -> Result<(u32, bool), String>
pub(crate) fn get_imm_range(ops, idx, lo, hi) -> Result<i64, String>
```

Then migrate the 209 `get_reg` / 47 `get_imm` call sites. This must be done
**file by file, with the assembler test suite green at each step** — not as one
mechanical sweep, because a mis-classified `get_gp_reg_or_sp` turns a valid
`ldr x0, [sp, #8]` into a hard error.

**Step 2 — the universal harness.** Add a new `encoder_pbt` test module beside
the existing encoder files and drive every mnemonic through the real
`encode_instruction` dispatch table and
assert the oracle-free invariants:

| id | invariant | catches |
|---|---|---|
| U1 | never panics, any operand tuple | F5 (42) |
| U2 | `xN → dN` must not produce an **identical** encoding | F1 (53) |
| U3 | `xN → wN` in a source position must not produce an identical encoding | F3 (113) |
| U4 | immediate `N` vs `N + field_modulus` must not produce an identical encoding | F4 (188) |
| U5 | deterministic | — |

U2–U4 need **no reference assembler** — which matters, because no `llvm-mc` or
aarch64 `gas` is installed. They work by observing that two semantically
different inputs produced the same bits, i.e. a field was ignored.

**Known false-positive sources — handle before trusting the report:**

- U3 fires on `uxtw x0, x1` (the W form is the canonical spelling of the same
  instruction). Whitelist or special-case the extend mnemonics.
- U4 must derive `field_modulus` per mnemonic; a wrong modulus produces noise,
  not signal. Start with the arithmetic-immediate and shift-amount families.
- U6 (`sp` vs `xzr` where no SP form exists) genuinely requires a per-mnemonic
  table. Land it as **data** in the repo, not as tests.

**Sequencing.** This is correctness hardening on a *secondary* target. The
standing brief ranks generated-code performance on x86-64 Raptor Lake first. Do
this after the x86-64 codegen queue, or in a session explicitly scoped to ARM.

### 4.3 P2 — consolidate the duplicated float converters

There are **two independent implementations** of f64→binary128 and f64→x87 in
the tree:

| | `src/common/long_double.rs` | `src/ir/constants.rs` |
|---|---|---|
| f64 → binary128 | `f64_to_f128_bytes_lossless` (L1158) | `f64_to_f128_bytes` (L54) |
| f64 → x87 | `f64_to_x87_bytes_simple` (L1280) | `f64_to_x87_bytes` (L109) |

This session fixed the same bug in all four. Two differential tests now pin them
together (`ir_constants_f128_matches_long_double_f128`,
`ir_constants_x87_matches_long_double_x87`), so a consolidation is now *safe to
attempt* and will be caught if it changes behaviour.

**But it is not byte-safe yet for specials.** Verified equivalence:

- binary128: zero, infinity, NaN (`1<<111` quiet bit) — **identical**; normals —
  **identical**. Consolidation is a pure deletion.
- x87: **NaN payloads differ.** `ir/constants` emits an all-ones mantissa
  (`0xFFFF_FFFF_FFFF_FFFF`); `long_double::make_x87_nan` emits
  `0xC000_0000_0000_0000`. Both are NaNs; the payload is observable.

So: delegate `f64_to_f128_bytes` → `f64_to_f128_bytes_lossless` unconditionally,
and for `f64_to_x87_bytes` delegate only after deciding (and testing) the NaN
payload policy.

### 4.4 P2 — `x87` subnormal decoding

`x87_bytes_to_f64` treats `biased_exp == 0` as either zero (mantissa 0) or
falls into the normal path with `unbiased = -16383`, which is **not** the x87
subnormal exponent (it is `-16382`, as in every IEEE format). Any x87 subnormal
therefore decodes wrongly. Reached only for values below ~3.6e-4932, so the
practical impact is tiny — but it is exactly the kind of thing that silently
corrupts an edge case for years. Use the `scaled_significand_to_f64` kernel
already added this session: it handles arbitrary (significand, exponent) pairs
correctly.

### 4.5 P3 — widen the differential oracle

`scripts/ldconst_differential.py` currently covers only `long double` globals.
The same shape works for:

- `_Float128` globals (exercises `const_to_f128_bits` in
  `src/ir/lowering/global_init.rs`, which calls `f64_to_f128_bytes_lossless` on
  `F64`/`F32`/`LongDouble` alike);
- `float`/`double` arrays with subnormal elements;
- Clang and ICX as additional references (both are `shutil.which`-gated already,
  so `--refs gcc,clang,icx` just works once installed).

Installing `clang` would have been free extra coverage for this session's
differential; it was not present on the image.

### 4.6 P3 — the fork's remaining non-encoder target-independent items

`#99` (LD1R post-index), `#210` (`sminv`/`uminv` opcode), `#321`/`#322` (SYS
`Rt` validation) are AArch64-encoder issues and fold into §4.2. No
target-independent issue from the corpus remains untriaged.

---

## 5. Reproduction commands for the next agent

```bash
export RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
export PATH=/opt/cargo/bin:$PATH
export CARGO_TARGET_DIR=/opt/lccc-target CARGO_BUILD_JOBS=2 CI_LOCAL_JOBS=1

cargo test --lib                                   # expect 4044 passed
python3 scripts/ldconst_differential.py --n 3000   # expect 3444/3444, 0 mismatches
./scripts/ci_local.sh --fast                       # must be green
LCCC_SNAPSHOT_UNGATED=1 ./scripts/lccc-snapshot.sh "<slug>" "<desc>"
```

Known-good state at end of session: base `070bf71e`, head `c14a10f7`, patch
`/home/user/ms178-1.patch`.

---

## 6. Open questions for the maintainer

1. **ARM encoder sequence.** §4.2 is a substantial, purely-correctness work item
   on a secondary target. Should it preempt x86-64 codegen performance work, or
   stay queued behind it?
2. **x87 NaN payload.** §4.3: keep `0xFFFF_FFFF_FFFF_FFFF` (ir/constants) or
   standardise on `0xC000_0000_0000_0000` (long_double)? GCC emits the latter.
3. **`lccc_delivery.py` budget.** §4.1: authorise modifying the census, or
   relocate the checkout out of `/home/user` instead?

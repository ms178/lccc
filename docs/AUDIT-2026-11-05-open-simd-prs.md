# Red-team audit of the open SIMD PRs (#1141, #1144, #1145)

Performed while #1146 (peephole borrow-instead-of-copy) was in CI. Every claim
below is labelled **reproduced**, **read**, or **not verified** — the distinction
matters more than the verdict.

Method: each PR head was fetched into `refs/remotes/pr/<n>` and #1141 was checked
out into a **detached worktree** at `/var/tmp/pr1141`, so the audit never touched
the delivery branch. Job logs are not downloadable from this sandbox (the Actions
blob host is unreachable), so failures were **reproduced locally** rather than
inferred from annotation text.

---

## #1141 — "SIMD implementation deep audit: 54 findings + 9-row reproducer ratchet"

`+7459/−42` in 23 files. Merge state **UNSTABLE**, ≥5 failing checks: Clippy,
Assembler and linker contracts, Codegen and runtime contracts (shard a), Fast
local contracts (shard b), Rust 1.99 relocation value contracts.

### Finding 1 — the Clippy lane fails on formatting. **REPRODUCED.**

```
$ cargo fmt --all -- --check   # in /var/tmp/pr1141
FMT_RC=1
8
Diff in src/backend/x86/assembler/encoder/avx.rs:4030
Diff in src/backend/x86/assembler/encoder/mod.rs:5246
Diff in src/backend/x86/assembler/encoder/mod.rs:5363
Diff in src/passes/arx_vectorize.rs:1036
Diff in src/passes/slp_vectorizer.rs:4607
Diff in src/passes/slp_vectorizer.rs:6397
Diff in src/passes/slp_vectorizer.rs:6407
Diff in src/passes/vectorize.rs:1238
```

The CI annotation is the same diff. `check_vex_half_width` was added at
module-level indentation inside a nested module, so rustfmt wants the whole body
re-indented. **Fix: `cargo fmt --all`.** One command; no design question. The PR
was pushed without running the formatter, which the Clippy lane checks first —
so the other four failures were never even reached on a clean build.

### Finding 2 — the new gates fail closed correctly. **REPRODUCED, and this is a strength.**

```
tests/simd/check_assembler_widths.py   rc=2  "lccc binary not found: target/fastbuild/lccc"
tests/simd/check_audit_2026-11-08.py   rc=2  "lccc not found: target/fastbuild/lccc"
tests/simd/check_simd_shape_matrix.py  rc=0  "12 flag sets, 29 shapes, 0 problems"
```

Exit **2** for "cannot run" rather than 0 is the right contract — a gate that
cannot execute must not report a pass. This is the same lesson as the vacuous
ratchet in #1146, applied correctly. The shape matrix passes standalone.

### Verdict

The audit content may well be valuable, but **the PR is not reviewable as landed**:
its first CI gate fails on whitespace, so none of the 54 claimed findings has been
validated by CI on this revision. Formatting must be fixed and the run repeated
before any claim in it can be trusted. I did **not** verify the 54 findings
themselves — that needs a built binary from that branch and is out of scope here.

---

## #1144 — "Harden SIMD forwarding against reconstructed and fixed addresses"

`+830/−90` in 7 files. Merge state **CLEAN** (all checks pass).

### The design change is right. **READ — I agree.**

`destination_unobserved_between` moves from a **blacklist** to a **whitelist**:

```rust
// before: everything is safe except these four
matches!(inst, Call | CallIndirect | InlineAsm | StackRestore)

// after: nothing is safe except these, explicitly
let safe = match inst {
    Alloca | Copy | GetElementPtr | GlobalAddr | LabelAddr | Phi | Select | ParamRef => true,
    BinOp { op, ty, .. } => !op.can_trap() && ty.is_integer(),
    Load { volatile: false, seg_override: Default, ptr, ty, .. }
    | Store { volatile: false, seg_override: Default, ptr, ty, .. } => { /* disjointness proof */ }
    _ => false,
};
```

**Why this is the correct direction.** A blacklist is only sound while the
instruction enum never grows. Every new `Instruction` variant would silently
default to "safe" — a fail-*open* default in a correctness proof. The whitelist
makes an unrecognised instruction refuse the optimisation, so adding a variant
degrades codegen instead of miscompiling. That is the same fail-closed principle
#1146 applies to its gate exit codes.

The disjointness proof for `Load`/`Store` is also sound as written:

```rust
ptr.0 != destination
    && ty.access_extent().is_some_and(|(_, may)| {
        may > 0 && allocas.get(&ptr.0).is_some_and(|info| {
            !info.semantic_volatile && (may as u64) <= info.size as u64
        })
    })
```

A **derived** pointer (GEP result, copy, bitcast) is not a key in `allocas`, so
`allocas.get` returns `None` and the arm fails closed — it does not fall back to
pointer-ID inequality. Using `access_extent()` rather than the object's declared
size is the right call and the comment explains why (i686 F128 constant stores
touch 16 bytes for a 12-byte `long double`). Checking `may <= info.size` bounds the
*whole* possibly-touched extent, not just its start.

### Finding — a pre-existing overflow shape is copied, but is unreachable. **READ.**

```rust
let count = component.iter().copied().max().unwrap_or(0) + 1;
```

`+ 1` on a `usize` maximum would overflow. The identical line already exists in
`main` (`vector_temp_promotion.rs:1430`), and `component` holds indices into a
rule vector, so `usize::MAX` is not constructible. **Not a defect worth changing**;
flagging it as one would be noise. Recorded so the next auditor does not spend
time on it.

### Verdict

Approve the direction. It is strictly safer than what it replaces and its CI is
green. Not verified: the claimed codegen deltas (needs that branch built).

---

## #1145 — "maskfacts: eliminate the runtime vpcmp* vector HOME roundtrip (7403 → 4523)"

`+1968/−18` in 11 files. Merge state **UNSTABLE** (Clippy pending at audit time).

### The operand-classification fix is correct and important. **READ — I agree.**

```rust
if let Some(width) = op.mask_compare_vector_width() {
    return (index < 2).then_some(width);
}
```

For AVX-512 `vpcmp*`, `args[0]`/`args[1]` are the vector operands and `args[2]` is
a **scalar predicate immediate**. Classifying position 2 as vector bytes would let
load forwarding replace `$imm` with an address — a hard miscompile. The `index < 2`
guard is exactly right, and routing it through one shared
`mask_compare_vector_width` table (rather than a second hand-written opcode list)
removes the drift risk between the folder, the alignment predicate and the emitter.

### Finding — #1144 and #1145 both edit `vector_temp_promotion.rs`. **READ.**

They touch **different functions** (`destination_unobserved_between` vs
`pointer_vector_arg_width_consumer`), and both report `MERGEABLE`, so there is no
textual conflict. The interaction is semantic, and it is benign:

* #1145 widens *which operand positions* may receive a forwarded load.
* #1144 narrows *which instruction windows* permit the promotion.

Both changes gate the same `promote_in_function` path, and **#1144's whitelist
does not admit `Instruction::Call`** — an intrinsic SIMD call therefore terminates
the observation window under either revision. #1144 cannot silently enable
something #1145 assumed absent, and #1145 cannot re-open a window #1144 closed.
The composition is the intersection of the two conditions, which is the safe
direction. **Not verified:** the combined codegen after both land; that requires a
merge build and the 7403 → 4523 measurement re-run on the merged tree.

### Verdict

The classification fix is right. The headline number is **not verified here** —
it must be re-measured on the merged tree, because #1144 changes the promotion
predicate that the roundtrip elimination depends on.

---

## Synthesis: what I take from these PRs, and what I do not

**Agree and adopt as a principle:** fail-closed defaults. #1144's whitelist and
#1141's exit-code-2 gates are the same idea, and #1146 applies it twice — the
ratchet returns 2 on an empty tree, and the i686 staging copy is reached through
an `expect` that turns a broken guard into a loud failure rather than a silent
match against `""`.

**Disagree with the process, not the content:** #1141 was pushed without
`cargo fmt`. A 7,459-line audit whose first CI gate fails on indentation has
*zero* CI-validated findings. The lesson is the one #1146 was built around: a
claim is worth exactly as much as the gate that actually ran.

**Do not repeat:** measuring a headline number on an unmerged tree when another
open PR changes the predicate it depends on. #1145's 7403 → 4523 must be re-taken
after #1144 lands.

**Toolchain note — checked, and *not* a defect.** CI reports
`gcc-16=16.0.1 (/usr/bin/gcc-16)` for most lanes while the tag cache holds upstream
**16.2.0**. This is deliberate: `ci_install_gcc16.sh` states "Debian must use Debian
packages, never an Ubuntu PPA", and a **separate** lane pins the exact upstream
version (`ci_install_gcc_oracle.sh --version 16.2`, the "Debian exact-version
oracle" workflow). The distro lanes test distro parity; the exact-version lane
tests upstream 16.2. Local work uses the tag-cache 16.2.0 as required. Changing the
distro lanes to 16.2.0 would destroy the parity coverage they exist for.


---

# Second pass, after rebasing onto `35a1e23`

Main advanced while #1146 was in CI: **#1144 was merged** (`35a1e23`), #1141 and
#1145 were force-updated, and **#1147 / #1148** were opened. Everything below was
re-measured on the new heads, not carried over.

## #1144 — MERGED

The whitelist rewrite of `destination_unobserved_between` landed as reviewed
above (`+543/−90` in `vector_temp_promotion.rs`). #1146 rebases onto it cleanly and
its ratchet, census and both test lanes were re-run on the merged base.

## #1141 — the formatting defect was fixed. **REPRODUCED FIXED.**

Re-fetched head `9074e79` (force-updated; retitled "SIMD perfection: audited
correctness fixes, red-team soundness proofs, 117 driver self-tests"):

```
$ cargo fmt --all -- --check    # /var/tmp/pr1141 @ 9074e79
PR1141_FMT_RC=0
0
```

Zero diffs, and `gh pr checks 1141` now reports **no failing checks**. The finding
was acted on. The 117 self-tests themselves are **not verified here**.

## #1147 — the identical defect, one PR later. **REPRODUCED.**

`+412/−6` in 2 files. Merge state **UNSTABLE**; Clippy, "Rust 1.99 relocation
value contracts" and the Test Suite aggregator all fail.

```
$ cargo fmt --all -- --check    # /var/tmp/pr1147 @ 9497b59
PR1147_FMT_RC=1
7
Diff in src/passes/copy_prop.rs:16
Diff in src/passes/copy_prop.rs:85
Diff in src/passes/copy_prop.rs:233
Diff in src/passes/copy_prop.rs:377
Diff in src/passes/copy_prop.rs:2776
...
```

The relocation lane's annotation carries **the same rustfmt diff**, so all three
failures have one root cause. **Fix: `cargo fmt --all`.**

**This is not a gate gap.** Both gates already exist and both fire correctly:

* `.github/workflows/ci.yml`, Clippy lane: `run: cargo fmt --all -- --check`
* `scripts/ci_local.sh:3637`: `gate "rustfmt" fast cargo fmt --all -- --check`

The formatter is the *first* thing the Clippy lane does, which is why it fails in
29 s. Two consecutive PRs pushed without running a gate that is present in both
CI and the local suite. The lesson is the one #1146's own CI failure taught the
hard way (see below): **run the cheap gates before pushing, not after**.

## #1145 / #1148 — no failing checks at re-check

`gh pr checks` reports no failures on either. #1145's `index < 2` operand guard
and the #1144 interaction analysis above still stand; now that #1144 is merged,
#1145's 7403 → 4523 claim should be re-measured against `35a1e23` rather than its
old base. **Not verified here.**

---

# The defect CI found in #1146 itself

Recorded because the same standard applies to my own work, and because it changed
the design for the better.

The first revision of `staging_line_text` used `.expect(...)` to make the
"every reader is guarded by an `s` parser" invariant loud. Two CI lanes failed:

* `Fast local contracts (shard b)` — `audit_census.py --check`
* `Assembler and linker contracts` — step 88, `check_param_home_contracts.py`,
  whose row **[12]** embeds the same census ratchet

Root cause, reproduced locally:

```
FAIL: audit census grew:
  panic_sites: 863 -> 864 (+1)
```

The census counts `.expect(` and `.unwrap(` as panic sites. Both lanes were green
on the base commit, so this was mine.

**Re-baselining would have been the lazy answer.** The `expect` guarded an
already-inert value: all eleven readers were audited individually and every one
returns `None` (or `""`) when handed `""` — ten start with
`strip_prefix("<mnem> ")?` / `split_once(' ')?`, and `imm_of` maps a missing
separator to `""` through `unwrap_or`. A pattern given `""` cannot match, so it
does not fire. A future unguarded reader therefore costs a **missed optimization,
never a miscompile**.

`unwrap_or("")` is strictly better: identical safety, no panic site, one fewer
branch in release. Verified after the change: census **PASS**, ratchet **PASS**,
34 contracts **OK**, `check_param_home_contracts.py` **13/13 rows PASS**, and CI
went **fully green** — `Assembler and linker contracts` and `Fast local contracts
(shard b)` both SUCCESS on the fixed head.

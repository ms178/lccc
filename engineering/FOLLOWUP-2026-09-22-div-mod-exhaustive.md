# Follow-up: exhaustive div/mod coverage, and an -O1 defect it exposed

Session of 2026-09-22 (third), rebased on `origin/main` `e08565c`.

Note on the base: `origin/main`'s tip is `e08565c` and it does **not** contain
the previous session's commits. The only part of this line of work merged
upstream is `05c9a6ba`. The previous patch was therefore re-applied on top of
`e08565c` before this session's work, so nothing was lost.

## 1. What was added

`tests/regression/div_mod_by_const_exhaustive.c` — exhaustive coverage for
`div_by_const`, the pass that replaces `div`/`idiv` (20-90 cycles) with
Hacker's Delight multiply-and-shift sequences. It had no exhaustive test, which
matters because a truncation bug in a neighbouring predicate
(`const_power_of_two` reading an `I128` constant through `to_i64`) was found in
this area recently.

Coverage: 24 unsigned-64 divisors, 15 signed-64 divisors plus their negatives,
14 unsigned-32 and 10 signed-32 divisors plus negatives — powers of two, one
below and above each power of two, small odd values that need the add-and-shift
fixup, magics needing the full 64-bit range, and the extremes. Each is crossed
with a dividend sweep of zero, the extremes, odd multiples, and the values
immediately around each multiple (where an off-by-one in the fixup shows).

The oracle uses **no division**. For every pair it asserts C's defining
properties of truncated division:

```
dividend == quotient * divisor + remainder
unsigned: remainder <  divisor
signed:   |remainder| < |divisor| and sign(remainder) == sign(dividend)
```

so a wrong quotient or remainder is caught without asking the compiler to
perform the operation under test. `INT_MIN / -1` is excluded because it
overflows the quotient and traps on x86.

Two design points that were wrong first and are worth recording:

* **The divisor must be a literal in each function.** The first version swept
  an array of divisors, which makes every division a *variable* division:
  `div_by_const` never fired and the test silently validated hardware `div`.
  Verified by perturbing the pass — changing the power-of-two shift from
  `d.trailing_zeros()` to `+ 1` left the array-driven test green. The current
  form generates one `noinline` function per divisor by macro. **A test that
  cannot fail is worse than no test**, so the perturbation check is what made
  this worth committing.
* **The checkers are side-effect-free.** They record the first offending case
  in globals and `main` reports it once. Calling `printf` from inside the
  checker put six computed values live across a variadic call at ~20000 sites.

State: suite **777 passed / 0 failed** (was 776). Green at `-O0`, `-O2`, `-O3`
and `-Os`; GCC agrees.

## 2. The defect it exposed at -O1 (FIXED)

At **`-O1` only** the test reported **341 failures, every one a power-of-two
divisor** (180 for `/8`, 161 for `/4`). The recorded case was self-refuting,
which is what made it worth chasing rather than filing:

```
FAIL u64 1 / 4: q=0 r=1, q*d+r=1
```

`q*d+r` is `1`, `n` is `1`, so the guard `r >= d || q*d + r != n` is false and
`record()` should not have been reached. The arithmetic was right; the branch
testing it was wrong.

### Root cause: `fold_lea_into_load` built an unencodable addressing mode

Localized with `CCC_PEEPHOLE_TRACE` (the skip-set bisection gives misleading
culprits, as the pass module itself warns). Dump `002-p0-fold_copy_shift_into_lea.s`
has the correct

```
leaq 0(,%r11,4), %r8        ; q*4
leaq (%r8, %r10, 1), %r9    ; + r
```

and dump `003-p1-fold_lea_into_load.s` has

```
leaq 0(,%r11,4, %r10, 1), %r9
```

which is **not a valid x86-64 operand**. A SIB byte carries one base (always
scale 1) and one index (scale 1/2/4/8); this has two index registers and no
base. The pass splices a producer `leaq`'s address into a later instruction
that reads it, and for the indexed consumer form `leaq disp(%T, %idx, %s)`
it substituted the producer's operand *text* into the base slot. That is sound
only when the producer address occupies the base slot itself (`disp(%base)`).
When the producer address carries its own index (`0(,%r11,4)`) the two index
fields land side by side.

### The miscompile is silent, which is why it survived

GAS rejects the line:

```
$ as --64 bad.s
bad.s:1: Error: expecting `)' after scale factor in `0(,%r11,4,%r10,1)'
```

lccc's integrated assembler instead **truncated** it to `leaq 0(,%r11,4), %r9`,
silently dropping `+ %r10`. So `q*d + r` became `q*d`, the program assembled,
ran, and printed plausible values. Only power-of-two divisors were affected
because only those make `q*d` a scale the folder can absorb — which is exactly
the signature the test reported.

### The fix

`compose_sib` replaces the text concatenation with a real addressing-mode
composition. It parses both operands into `SibAddr { sym, disp, base, index }`,
merges repeated registers, and emits the sum only when the SIB byte can carry
it: at most two distinct registers, at most one of them scaled, scale in
{1,2,4,8}, base unscaled. The `+ %r10` term is **kept**, and the result stays
at one instruction:

```
leaq (%r10, %r11, 4), %r9     ; one index register, one base, valid, GAS-clean
```

That is what GCC and Clang emit for the same source. When the sum is *not*
representable (three registers, two scaled registers) the fold is declined and
both instructions are kept — as before, but now for the right reason rather
than by counting registers in a way that did not match what the splice did.

Two secondary findings from the red-team pass over the first fix, both fixed:

* The first version `continue`d out of the window-scan loop on a declined
  fold, which skipped `window += 1` and silently extended the pass's scan
  window. The original control flow (decline, then `break`) is restored.
* Extracting the consumer's displacement as "everything after the mnemonic"
  breaks when the matched operand is not the first one — a store such as
  `movq %rax, (%rcx,%rbp,8)` has a register operand before it. `operand_start`
  now steps back to the operand separator. This one was caught by a unit test
  (`indexed_store_still_folds`) written specifically to probe it.

Symbolic displacements are carried through rather than rejected
(`leaq table(%rcx), %r8` into `8(%r8,%rbp,8)` gives `table+8(%rcx, %rbp, 8)`),
because dropping them would have silently narrowed an optimization the pass
documents as its motivating case. A scale of 1 renders implicitly
(`8(%rsp, %r8)`, not `8(%rsp, %r8, 1)`) to preserve the emitter's canonical
form — a change to this was caught by
`dead_writes::sib_destination_is_memory_not_a_fake_index_register`.

### Verification

* `tests/regression/lea_chain_index_compose.c`, pinned to `-O1`, is
  mutation-proven: **exit 1 on the old binary, exit 0 on the fixed one**, and
  0 under GCC. It covers scales 2/4/8 and 16, signed variants, non-power-of-two
  controls whose product is not a scale, and a symbol-indexed gather.
  Two shape requirements are documented in the file because they were
  measured, not assumed: the divisor must reach the division as a *parameter*
  that constant-folds through inlining, and the `printf` in the failure branch
  is load-bearing — it provides the register pressure that produces the lea
  chain the fold then mangles. Removing it makes the miscompile vanish rather
  than merely go unreported.
* 21 new unit tests around `SibAddr::parse`, `compose_sib`, `operand_start`
  and the end-to-end driver; the full unit suite is 3152 passed / 0 failed.
* `div_mod_by_const_exhaustive`: 341 failures -> **0** at `-O1`, ALL-OK at
  `-O0`/`-O2`/`-O3`/`-Os`.
* Regression suite 778 -> **779 passed / 0 failed**.

### The second defect: the assembler truncated instead of erroring

Fixing the peephole alone would leave the landmine armed. `parse_memory_inner`
in `src/backend/x86/assembler/parser.rs` split the parenthesised operand on
commas, took `parts[0..3]` as base/index/scale, and **silently ignored any
further fields**:

```rust
let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
let base  = if !parts.is_empty() ... ;      // parts[0]
let index = if parts.len() > 1 ...;         // parts[1]
let scale = if parts.len() > 2 ...;         // parts[2]
// parts[3..] dropped
```

That is what turned an invalid operand into a wrong-code miscompile rather
than a compile error: `0(,%r11,4, %r10, 1)` assembled as `0(,%r11,4)`. GAS
rejects the same input with `expecting ')' after scale factor`. A compiler
assembler that accepts syntactically valid text and emits *different*
semantics is worse than one that refuses it, so the parser now rejects a
fourth field:

```
ccc: error: line 1: memory operand has 5 comma-separated fields inside the
parentheses, but at most base,index,scale are encodable: (,%r11,4, %r10, 1):
'leaq 0(,%r11,4, %r10, 1), %r9'
```

With this in place the whole class is loud: any future pass that builds an
unencodable operand fails the build instead of shipping a silent miscompile.
Four unit tests cover the rejection and the surviving canonical forms, and the
full unit suite (3155) confirms no valid code used a four-field operand.

### Bug-class audit

Every site in `src/backend/x86/` that pattern-matches a *partial* indexed
operand (the `"(%T,"` signature that made the concatenation unsound) was
enumerated: `relay_and_lea.rs:1205` is the **only** one. The other address
folds in `local_patterns.rs` and `memory_fold.rs` match a *complete* `(%T)`
operand and replace it with another complete operand, so they cannot
concatenate two SIB expressions. The defect does not recur elsewhere.

### Performance: the fix is free

`scripts/census_ab.py` (the documented screening gate) over the full
regression corpus:

| corpus | functions | insns | rrmov | stkref | push | acc |
|---|---|---|---|---|---|---|
| 748 files, `-O2` | 3134 | 0 | 0 | 0 | 0 | 0 |
| 748 files, `-O1` | 4374 | 0 | 0 | 0 | 0 | 0 |

Zero delta in every column at both levels, and the gate passes. Text-level
diff of the new test file at `-O1` shows exactly the intended change:

```
-    leaq 0(,%r11,4, %r10, 1), %r9     (4 occurrences: scales 4 and 8)
+    leaq (%r10, %r11, 4), %r9
```

Two-index operands 4 -> 0, **instruction count identical (471 = 471)**: the
fold still eliminates the producer `leaq`, so the correction costs nothing.
The census columns are also why the earlier claim of an unchanged corpus is
precise about what was compared -- instruction counts and register traffic,
not operand text.

## 3. Measured: dead frames

483 of 1412 functions that reserve a frame (**34.2%**) reserve one that is
**never referenced by any instruction** — 15,200 bytes, and two wasted
instructions (`subq`/`addq`) per function. 478 of those have a frame of 256
bytes or less, so they are spill reservations whose slots all ended up in
registers, not arrays addressed through `leaq`.

`div7` is a clean example: `pushq %rbx` plus `subq $112, %rsp` for a function
that touches no memory at all, against GCC's frameless 7 instructions.

`peephole/passes/frame_compact.rs` already exists and is the natural home for
this. Not attempted this session: removing a frame changes `%rsp` at every call
site, so it needs the alignment argument and the kernel boot gate done
properly, and the linux-cachymod tree is not present in this workspace.


## 4. CI was red: rustfmt, and the missing trace-bisect tool

The revision that went up as PR #592 failed CI. The cause was `cargo fmt
--check`, which is a hard gate (the `clippy` job runs rustfmt with
`-D warnings`), and four formatting diffs had slipped through:

* a stray blank line before a module close in `src/backend/x86/assembler/parser.rs`;
* a needless line break in `emit_movabs_to_r8`;
* two over-wrapped `assert_eq!`/`assert!` calls in the new `relay_and_lea`
  tests.

These were invisible locally because I had been running the regression corpus
and the unit tests, but not the lint gates. `cargo fmt` now leaves the tree
unchanged and `cargo fmt --all -- --check` passes; `cargo clippy
--all-targets --profile fastbuild --locked -j 1 -- -D warnings` is clean. `--fast`
skips only three gates and **never** rustfmt or clippy, precisely so that a red
PR is reproducible before it is pushed.

### The comment named a script that did not exist

While auditing what CI checks, one of the peephole driver's comments turned out
to promise a tool that was not in the tree:

```rust
// The skip-set bisection (CCC_PEEPHOLE_SKIP) gives MISLEADING culprits when
// passes enable one another ... The trace is the reliable instrument:
// assemble+run consecutive dumps and the first one that breaks the program
// names the faulty rewrite (`scripts/peephole_trace_bisect.py` automates it).
```

That absence is why the `fold_lea_into_load` miscompile had to be bisected by
hand -- and why `CCC_PEEPHOLE_SKIP` named eight different "culprits" for one
bug before `CCC_PEEPHOLE_TRACE` pointed at dump 003. The script now exists,
with two checks per dump:

* **assemble** -- hand the dump to a real assembler. A pass that emits an
  instruction with no encoding fails here, which covers the whole class of
  mis-encoded-operand bugs rather than only this one.
* **operands** -- parse the dump's memory operands for base/index/scale fields
  that cannot be encoded.

Both are needed: the integrated assembler *silently truncated* the unencodable
operand, so assembling is not sufficient on its own.

The operand check is a parser, not a regex, deliberately. A first attempt with
one pattern silently missed the empty-base form `(,%r11,4, %r10, 1)` -- exactly
the shape the historical bug produced. A 21-case self-test pins every shape,
including **seven correct forms that must not be reported**: a false positive
names the wrong pass, which is as harmful as a miss.

Gated two ways, because a tool nothing runs rots:

* `tests/regression/check_peephole_trace_bisect.sh`, a fast gate that runs the
  self-test, proves the self-test *can fail* (by inverting an expectation and
  requiring a non-zero exit), drives a real compile end to end, and asserts the
  **content** of the clean verdict rather than only its exit code -- a first
  version of this gate read a blank line and would have passed a tool that
  printed nothing;
* `python3 scripts/peephole_trace_bisect.py --selftest` directly in the hosted
  workflow, so a broken parser fails CI even if the wrapper is skipped.

`check_ci_gate_parity.py` requires that any `scripts/*.py` named in
`ci_local.sh` -- *including in a comment* -- is actually executed by a workflow
`run:` block; it reported the new tool until the workflow ran it too, and now
passes with 53 commands.

## 5. Green, and how it was verified

Every gate that can run on this host was run, including the three `--fast`
skips:

| gate | result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --all-targets ... -- -D warnings` | PASS |
| `cargo-test` (unit suite) | PASS -- 3155 passed / 0 failed |
| `ci_local.sh --fast` | **60 passed, 0 failed, 3 skipped** |
| `regression-corpus-ssa` (`CCC_VALIDATE_SSA=1`) | PASS -- 779 passed / 0 failed |
| `benchmark-output-oracle` | PASS -- 204 passed / 0 failed |
| `peephole-whitespace-invariance` | PASS (4 phases, 228 generated files) |
| `ci-gate-parity`, `doc-link-integrity` | PASS |

The three slow gates matter here because two of them cover this change
directly: the SSA corpus gate exercises the codegen path, and the whitespace
gate exercises the peephole pass that was modified.

## 4. Measured dead end: the `mulhi` cross term

`div_const` is 20 instructions against GCC's 7. The obvious remaining waste is
the `lhs.hi * lo` cross term in the 128-bit multiply, which is identically zero
for strength-reduced division because the left operand is `(u128)x`.

An emission-time fast path was implemented: `operand_to_rax_rdx` records
whether the load left `%rdx` at zero, and `emit_i128_mul_const` drops the term
when it did. **It cannot work, and the reason is specific.** Tracing showed the
left operand is an i128 value *with a stack slot*, so the backend emits two
slot loads and correctly reports "high half unknown"; a **peephole** later
rewrites the high-half load into `xorl %edx, %edx` because it can see the
stored half was zero. The fact only exists after emission, so no emission-time
flag can observe it. The code was reverted rather than shipped inert.

Two viable directions, in order of promise:

* A peephole that propagates a known-zero register: `xorl %edx,%edx` then
  `movq %rdx,%rcx` makes `%rcx` zero, so `imulq %r8,%rcx` is zero and
  `addq %rcx,%rdx` is dead. Block-local, generally useful, and it is where the
  information actually lives.
* An IR-level `mulhi` op so the 128-bit product is never materialised. Bigger,
  and it is what GCC and Clang effectively do.

## 5. State

Suite 776 → **777 passed / 0 failed**. Deliverable re-based on `e08565c`.
Kernel build/boot gate still not re-run — the linux-cachymod tree is absent.

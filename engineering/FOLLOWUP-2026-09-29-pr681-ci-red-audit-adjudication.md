# PR #681 CI-red: audit adjudication, the volatile regression, and what it exposed (2026-09-29)

Base `02d4c906` (= `origin/main`), PR head `6358140a`. Everything below was
measured on a 2-core / 2 GB KVM host with a 3 GB swapfile; `lccc` built with
`scripts/build_lccc_fast.sh` (fastbuild, `-O1`, `-j2`).

---

## 1. What was broken, and what is now true

| # | Finding | Verdict | Evidence |
|---|---|---|---|
| BLOCKER-1 | corpus glob collision | **confirmed** | `run_regression.py:142` globs `*.c` non-recursively; both fixtures failed with `undefined reference`. Reproduced: `2 passed, 2 failed`. After the fix: `4 passed, 0 failed`. |
| HIGH-1 | `!*volatile` deleted from LICM | **confirmed** | `while (!regs[4]) {}` compiled to a load **outside** `.LBB1`, i.e. an infinite loop. After the fix the loop body is `movl 16(%rdi), %edx; testl; je .LBB1` — identical to `gcc -O2`. |
| MED-1 | vacuous PASS on empty oracle set | **confirmed** | `reloc_oracle_agreement([], {}, [])` returned `('PASS','')`. Now `('FAIL','no oracles configured to cross-check against')`. |
| MED-2 | cache ignores compiler drift | **confirmed** | No `version` anywhere in `tools/oracle/godbolt_oracle.py`. Now: per-record provenance + `--revalidate`. |
| MED-3 | promised preheader gate absent | **confirmed** | `check_loop_preheader.sh` / `loop_preheader_shapes.c` did not exist. Both now exist and are wired. |
| LOW-1 | `errors="replace"` betrays the corruption contract | **confirmed** | strict decode + 2 new self-test checks. |
| LOW-2 | `stats()` temp filter is dead code | **confirmed** | `".tmp." in name` + 1 new self-test check. |
| LOW-3 | 105-line verbatim twin | **confirmed, mechanically** | Diffed the two blocks: 105 lines, **0 differences**. Extracted to `hoist_dynamic_limit`. |
| LOW-4 | `LATE_MINMAX_ONLY` thread-local | **confirmed** | Left as-is; see §5. |
| INFO-2 | journal trailing blank line | **confirmed** | `\n\n` at EOF. |

## 2. Where the Review AI was wrong

It had no way to run anything, and it shows in three places. All three matter,
because two of its nine "instructions for the follow-up agent" would have made
things worse.

### 2.1 Its safety net does not exist (this is the important one)

The audit says:

> `tests/regression/check_volatile_pointer_subscript.sh` exists in the base and
> asserts *exactly* this failure mode (`FAIL spin: load hoisted out of the
> volatile spin loop`).

It does not. The assertion was

```sh
spin_loads=$(count_mem_movs spin)      # memory movs ANYWHERE IN THE FUNCTION
spin_branch=$(... j(e|ne|z|nz) ...)
if [ "$spin_loads" -lt 1 ] || [ "$spin_branch" -lt 1 ]; then fail; fi
```

and the buggy code satisfies both terms: the hoisted load is still *inside
`spin`*, and `je .LBB1` is still a conditional branch. **Measured: the gate
printed `volatile pointer subscript / arithmetic: ok` and exited 0 with the
miscompile live in the binary.** The audit's step 3 ("wire the gate") would
therefore have wired a gate that cannot fail on the bug it was wired for.

The fix is not the wiring, it is the assertion: the load must be inside the
**backward-branch body**, not inside the function. That is now
`mem_accesses_in_loop_body`, and it is mutation-verified in both directions
(delete the guard → `FAIL spin: no memory access inside the loop body`;
restore it → pass).

### 2.2 The gate it did not mention was the one that noticed

`check_volatile_access_semantics.sh` — not mentioned in the audit — *was*
failing, with exactly the right message:

```
FAIL: loop keeps volatile load in body (pattern 'mov(l|slq) +[^,]*\(%[re]?[a-z0-9]+' not in reads_in_loop)
```

at `-O2` and `-Os`. But it was failing for a **second, independent reason**, and
the two had to be separated before either could be fixed:

* `int reads_in_loop(int n){ int t=0; for(...) t += counter; }` selects the
  fused read-modify-write `addl counter(%rip), %esi`. That is a correct
  volatile read, re-executed every iteration, and one instruction tighter than
  GCC's `movl counter(%rip), %ecx` + `addl %ecx, %edx`. The test's `mov`-only
  pattern called it "eliminated".
* The pattern is now `read_pat` (any ALU op with a trailing memory operand;
  AT&T puts the memory operand last, which is what keeps a *store* out of the
  match). 20/20 checks pass at O0/O1/O2/Os.

With that fixed, the gate still does **not** catch the LICM hoist (it is
function-scoped by design), so `check_volatile_pointer_subscript.sh` remains the
catcher. The division of labour is now documented in both files.

### 2.3 Wiring both gates, as instructed, would have turned CI red

`check_volatile_access_semantics.sh` was in `scripts/ci_gate_allowlist.txt`
marked `# exits 1 after partial pass — needs triage`. Wiring it before fixing
the pattern would have failed the Test Suite job on a correct compiler. Order of
operations mattered here: fix the pattern, then wire.

## 3. What the audit did not look at, and what fell out

### 3.1 `loop_preheader` does not do what its own documentation says

The module doc motivates the pass with

```c
int f(const int *c, int n) { int t = 0; for (int i = 0; i < n; i++) t += c[0]; return t; }
```

Measured: `CCC_DEBUG_LOOP_PREHEADER=1` prints **nothing** for that function, and
`CCC_DEBUG_LICM=1` explains why —

```
[LICM] load v14 in block 2 not hoisted: its block does not dominate every loop
block (block is not the header, so the load is not must-execute)
```

The pass's own profitability gate (`preheader_would_unlock_a_hoist`) mirrors
LICM's must-execute rule, so it declines the loop whose shape the docstring uses
as the reason the pass exists. The pass is not broken; the example is wrong. A
do-while *is* unlocked (`dw_hoist`: `movl (%rdi), %edx` moves above the loop),
because there the frontend puts the load in the header.

Worse, in the SQLite shape that the pass *does* fire on, only the loop **bound**
is hoisted. Steady-state memory operands in the loop body:

| | bound `p->nUsed` | accumulator `p->v` |
|---|---|---|
| pass ON | hoisted (0) | **1 per iteration** |
| pass OFF | 1 per iteration | 1 per iteration |

So the pass delivers 2 → 1, not 1 → 0. That is a real win and it is now pinned
exactly that way (see `check_loop_preheader.sh` contract 2), with the gap filed
as LOOP-PREHEADER-3 below rather than papered over.

### 3.2 A 4-instruction waste in bottom-tested loop backedges

```c
int f(int n){ int i = 0; do { i++; } while (i < n); return i; }
```

```
lccc -O2:                          gcc -O2:
    xorl %esi, %esi                    testl %edi, %edi
  .LBB1:                               movl $1, %eax
    addl $1, %esi                      cmovg %edi, %eax
    cmpl %edi, %esi                    ret
    setl %r8b          <-- 4 instructions
    movzbl %r8b, %r8d  <-- to do the
    testb %r8b, %r8b   <-- work of one
  jne .LBB1            <-- conditional jump
```

The loop condition is materialised as an `i1`, zero-extended to `i32`, then
`test`+`jne` — while the compare that produced it sits right there, unused as a
branch.

**Measured scope, not assumed.** Eight shapes, `-O2`, counting `setCC` inside
the loop body:

| shape | `setCC` in loop |
|---|---|
| `do { i++; } while (i<n); return i;` | 1 |
| same, returning a constant | 1 |
| `do { s+=i; i++; } while (i<n);` | 1 |
| `do { i+=2; } while (i<n);` | 1 |
| `while (i!=n)` / `while (i<=n)` spelling | 1 / 1 |
| `for(;;){ i++; if(!(i<n)) break; }` | 1 |
| `while (i<n) { i++; }` (top-tested) | **0** |
| `do { t+=a[i]; i++; } while (i<n);` (marching pointer) | **0** |

So the trigger is a **bottom-tested** loop whose exit condition stays an index
compare; top-tested loops and loops the vectorizer turns into marching-pointer
form are already fine.

**This is not the rotation item the journal already filed.** `2026-09-W3.md`
records the same `setl; movzbl; testb; jne` sequence as a reason `loop_rotate`
stays opt-in. Checked, and the attribution does not carry over: `loop_rotate`
is opt-in (`CCC_LOOP_ROTATE=1`), and re-measuring all eight shapes with
`CCC_DISABLE_PASSES=loop_rotate` leaves every count **unchanged**. So the
default `-O2` pipeline, with no environment variables set, pays this on
bottom-tested loops. Same symptom, different pass, and this one is already
shipping. Across the first 60 `tests/regression/*.c` files, 9 emit
at least one `setCC` (not all of them in a loop). This is
**DO-WHILE-BRANCH-1** below: 3 wasted instructions and 1 wasted uop per
iteration in the steady-state body, on a shape the Linux kernel uses heavily
(`do { } while (likely(...))`).

### 3.3 A portable-shell trap that silently disabled a gate

`mawk 1.3.4` (default `awk` on Debian/Ubuntu, therefore on the CI image) types
an *uninitialised* variable as the empty string:

```sh
$ echo 'BEGIN{best=0;loops=1;n=0;best=(n>best?n:best)} END{print (loops>0?best:-1)}' | awk -f -
0
$ echo 'BEGIN{loops=1} END{print (loops>0?best:-1)}' | awk -f -
            # <-- blank line, not 0
```

My first version of the loop-body parser hit exactly this and printed a blank
line, so `[ "$x" -lt 1 ]` died with *integer expression expected* and the gate
**passed**. Both new parsers now initialise in `BEGIN` and the shell side
validates that the answer is numeric before comparing, failing loud on garbage.

### 3.4 The parser heuristic was wrong for rotated loops

The first loop-body parser counted between a label and the branch that targets
it. For the rotated shape lccc emits for a guarded loop the back-edge is an
unconditional `jmp` at the **bottom** and the body sits **above** its target:

```
.LBB5:  cmpl (%rdi), %r9d    <- body starts at the jmp target
        jge .LBB7            <- forward exit, not a back-edge
.LBB6:  addl 4(%rdi), %r8d
        jmp .LBB5            <- the back-edge
```

Both parsers now collect the function body and compute the span
`pos[target] .. back-edge` in `END`, taking the maximum over all loops.

### 3.5 Minor

* `check_volatile_access_semantics.sh` used `set -uo pipefail` (no `-e`) and
  `grep -c` without `|| true`; under `-e` a zero count would abort the script
  instead of reporting one failed check. Both fixed, plus a numeric guard.
* Dead code in the same file: `body=$(awk ... | awk ...)` whose result was
  never used (`check` re-extracts it). Removed.
* `wr_cond` emits `testl %edx, %edx` twice (`cmovneq %rdi, %r8` then
  `cmovneq %rsi, %rdi`): the second select re-tests a live condition instead of
  reusing the first result. Pre-existing, unrelated to volatile; filed below.

## 4. Why the volatile deletion was invisible

**This section originally gave the wrong cause, and the correction is the most
important finding in this document.** The earlier text asserted that `rustc`'s
`unused_variables` does not fire on a binding introduced by refutable pattern
destructuring. That is false, and it was inferred rather than tested: the
observation was real (with `!*volatile` deleted, `./scripts/build_lccc_fast.sh`
— which passes `-D warnings` — finished clean) but the explanation was not.

The real cause is one line: `src/lib.rs` carried

```rust
#![allow(dead_code, unused_variables, unused_mut, unused_assignments,
         unused_imports, unreachable_code)]
```

A crate-level `allow` silences the lint at its source. A command-line `-W`
cannot override it, which is why `-D warnings` in the build script changed
nothing, and — verified separately — why `cargo fix` reports "89 suggestions"
while applying none of them: machine-applicable suggestions are filtered by the
same lint level.

Measured directly, with the allow removed and the guard deleted, rustc reports
the bug precisely:

```
warning: unused variable: `volatile`
    --> src/passes/licm.rs:1256:30
     |
1256 |                     ptr, ty, volatile, ..
     |                              ^^^^^^^^ help: try ignoring the field: `volatile: _`
```

It names the file, the line, the binding and the fix. So the layers that
failed were:

* the compiler **could** see it, and was told not to say anything;
* clippy inherits the same level, so it was silent for the same reason;
* the one gate written for it could not fail on it (§2.1);
* and that gate was not wired anyway.

One suppressed lint, three consequences. **Fix applied:** `unused_variables` is
removed from the crate-wide allow and all 99 resulting warnings are resolved —
87 by prefixing the dead binding with `_`, 7 by spelling a struct-shorthand
field as `field: _field`, 3 `let mut` bindings by hand, and 2 items annotated
with a per-item `#[allow(unused_variables)]` plus a comment (the two disabled
`try_emit_phase9_indexed_*` functions in `src/backend/x86/codegen/memory.rs`,
whose bodies are kept as executable documentation and therefore reference
parameters that are genuinely never read). `unused_variables` is the only lint
removed from the allow; the others are untouched.

The scope difference matters and is the reason to prefer this over a
hand-written checker: a static ratchet written for `volatile` covers
`volatile`, while the lint covers **every** destructured safety flag in the
crate — aliasing, atomicity, `noalias`, signedness, anything added later — for
free, with span-accurate diagnostics. It does have one blind spot, stated here
so it is not mistaken for complete coverage: `if false && *volatile` still
*reads* the binding, so the lint stays quiet while the guard is dead. Deleting
the term entirely (the actual PR #681 bug) is caught. Both the lint and the
ratchet are kept, because they fail on different mistakes.

The lesson recorded in `licm.rs` at the fix site is unchanged: the volatility
check is spelled as its **own branch with its own `licm_debug` message**, not
folded into a conjunction, so that deleting it deletes a visible, named thing.

## 5. Decisions taken, and one deliberately not taken

* **Refuse to "fix" the corpus by moving files alone.** The audit's step 1 was
  `git mv` both fixtures into `minmax_shapes/`. That silences the corpus but
  deletes coverage, and `minmax_refused_main.c` says in its own header that
  being corpus-run was the point. Instead: the multi-TU files moved (they are
  genuinely not standalone TUs) and two thin root-level wrapper TUs
  (`minmax_reduction.c`, `minmax_refused.c`) `#include` them, so the randomized
  differential and the refused-shape checksum now run on **every** corpus
  invocation at `-O2` with no oracle. Harness runtime measured at 0.18 s.
* **Teach the contract instead of adding an exclusion list.** `run_regression.py`
  now recognises `undefined reference` in a compile failure and prints the
  single-TU rule plus a worked example. An `.skip`/`extra-sources` convention
  was considered and rejected: it is a mechanism for hiding.
* **MED-2: provenance + out-of-band revalidation, not version-in-key.** The
  audit preferred this and its reasoning holds, but it stopped short of the
  actual problem: for `cicxlatest` CE reports `(latest)`, so there is no version
  to compare. The mechanism is therefore two-pronged — semver drift for pinned
  channels, `--max-age-days` (default 28) for moving ones — driven by ONE
  `/api/compilers` request per run. Verified live (`cg162`→`16.2`,
  `cclang2310`→`23.1.0`, `cicxlatest`→`(latest)`) and offline (3 unit tests:
  drift/age/legacy dropped, pinned-old kept, failed probe keeps everything,
  `--max-age-days 0` drops only the unversioned).
* **LOW-4 not taken.** Converting `LATE_MINMAX_ONLY` to a scope guard is
  self-consistent but touches the pass driver for zero behavioural change, and
  this session's budget went to a live miscompile. It stays open, honestly
  labelled as style rather than risk.

## 6. Follow-up work, prioritised

Ranked by `expected impact × affected workloads ÷ implementation cost`.

### DO-WHILE-BRANCH-1 — high value, low risk *(new this session)*

**Symptom.** Bottom-tested loops emit `setCC / movzb / test / jne` where a
single `jCC` suffices (§3.2). Three redundant instructions and one redundant
uop per iteration, in the steady-state body.

**Measured scope.** All six bottom-tested shapes tested show it; the top-tested
`while` and the marching-pointer do-while do not. 9 of the first 60 corpus files
emit at least one `setCC`.

**Reproducer.** `int f(int n){int i=0; do{i++;}while(i<n); return i;}` at `-O2`.
No pointers, no `main`, no specialisation involved.

**Where to look.** The lowering of a bottom-tested exit produces an `i1` that a
branch then consumes. Either (a) the frontend emits the conditional branch
directly, or (b) a peephole fuses `Cmp → SetCC → ZExt → Test → CondBr` back
into `Cmp → CondBr`. (b) is the smaller change and catches the pattern wherever
it arises; (a) is the better long-term shape. The machinery already exists —
the top-tested path and the marching-pointer path both emit `cmp` + `jCC`
directly — so this is a missing case, not a missing capability. Start by finding
which lowering path the bottom-tested exit takes that the other two do not.

**Gate to add.** Assembly-level: a bottom-tested loop's body must contain no
`setCC`, and its backedge must be a single `jCC`. Root-level standalone TU (see
the BLOCKER-1 lesson). Write the gate first and let it stay red until the fix
lands — that is what `ci_gate_allowlist.txt` is for.

**Expected win.** ~3 instructions per iteration on every bottom-tested loop.

### LOOP-PREHEADER-3 — the pass under-delivers *(new this session)*

**Symptom.** In the guarded (`if (p == 0) return;`) shape the pass makes the
preheader dedicated and LICM hoists the loop **bound**, but the loop **body**
load (`p->v`) is still refused, because LICM's must-execute rule requires the
load's block to dominate every loop block and the load sits in the body, not the
header. 2 memory operands per iteration instead of 1.

**Why it is safe to relax.** If a block is *dominated by the header*, then
entering it is conditional only on path conditions **inside** the loop. The
loop-exit branch cannot be taken before the body's first instruction has run, so
a load in a header-dominated block still executes at least once whenever the
loop is entered — which is the entire property the dedicated preheader buys.
The current rule ("must be the header") is a special case of the rule that is
actually needed ("must be dominated by the header").

**Risk.** This widens a gate that exists to prevent a segfault, so it needs the
SQLite fixture, the `20051215-1.c` guarded-deref torture shape, and a
differential sweep before it lands. Not a drive-by.

**Then.** Tighten `check_loop_preheader.sh` contract 2 from `-ne 1` to `-ne 0`.
The gate is written so that this is a one-token edit — that was deliberate.

### LOOP-PREHEADER-4 — the docstring lies *(documentation)*

*(Numbered -4: LOOP-PREHEADER-2 is already taken by the `loop_rotate`
default-enable item in `engineering/journal/2026-09-W3.md`.)*

`src/passes/loop_preheader.rs` motivates the pass with a counted loop it does
not fire on (§3.1). The example should be the do-while, with the counted loop
kept as an explicit *counter*-example. `loop_preheader_shapes.c` already carries
all three shapes with the reasoning inline; the module doc should point at it.

### WR-COND-RETEST — small, pre-existing

`wr_cond` re-tests a live condition for the second conditional store instead of
reusing the first `cmov` result (§3.5). One redundant `test`. Worth a look while
someone is in select/peephole for DO-WHILE-BRANCH-1.

### MINMAX-2 / MINMAX-4 — unchanged from the PR

Multi-accumulator refusal (`sum == 0` miscompile avoided by refusing) and the
late-rerun scope limited to min/max. Both were correctly decided and correctly
documented by the PR; the audit agreed and so do I. The callgrind number
(+1.05% adler32 for the wider late rerun) is the right reason to have declined.

### Cache hygiene — INFO-1 from the audit

`godbolt_cache.key()` reproduces the legacy `att-v2` digest under new paths, so
pre-existing flat `*.s` entries orphan rather than upgrade. One network
re-fetch; a one-time cleanup script would be tidy but is not worth a gate.

## 7. Validation actually run

| Check | Result |
|---|---|
| `run_regression.py --filter minmax` | 4 passed, 0 failed (was 2/2) |
| `run_regression.py --filter loop_preheader` | 1 passed |
| `check_minmax_reduction.sh` | PASS, 14 shapes, per-shape insn/byte counts unchanged after the LOW-3 extraction |
| `check_volatile_pointer_subscript.sh` | pass on the fix; **FAIL on the reverted guard** (mutation-verified) |
| `check_volatile_access_semantics.sh` | 20/20 at O0/O1/O2/Os |
| `check_loop_preheader.sh` | pass; asserts insertion count, the ON/OFF delta, and guard-before-load |
| `test_reloc_oracle_verdict.py` | 32 cases (was 30) |
| `test_godbolt_cache.py` | 13 functions / 23 assertions (was 13/20) |
| `godbolt_oracle_selftest.py` | 14 tests (was 11) |
| `check_ci_gate_parity.py` | PASS, 100 commands |
| reduction gate battery (8 scripts) | 6 wired gates PASS; the 2 failures are pre-existing allowlisted orphans |

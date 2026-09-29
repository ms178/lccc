# PR681-AUDIT-RESPONSE — evidence

Everything here is a **measurement**, not a claim. The narrative and the
reasoning are in
[`engineering/FOLLOWUP-2026-09-29-pr681-observable-access.md`](../../FOLLOWUP-2026-09-29-pr681-observable-access.md);
this file holds the artifacts.

Base: `main` `02d4c906`. Branch: `arena/01a0ee1a-lccc` (PR #681, head
`6358140a`).

---

## E1 — the LICM volatile guard deletion is a real miscompile

**Mutant:** `src/passes/licm.rs`, Load arm of `hoist_loop_invariants`, the
`if *volatile` branch replaced by `if false` (exactly PR #681's shape, which
dropped the term during the if/else-if/else refactor). Built with the project
policy (`scripts/build_lccc_fast.sh`: fastbuild, `-O1`, `-j 2`).

**Probe:**

```c
typedef unsigned int u32;
u32 spin(volatile u32 *regs) { while (!regs[4]) { } return 0; }
```

**Emitted assembly, mutant:**

```asm
spin:
    # LCCC_RET_XMM 0
    # LCCC_RET_RDX 0
    # LCCC_RET_RAX 1
    movl 16(%rdi), %edx     <-- hoisted: one read, then spin on a frozen value
.LBB1:
    testl %edx, %edx
je .LBB1
.LBB2:
    xorl %eax, %eax
    ret
```

**Emitted assembly, fixed** (load back inside the loop):

```asm
spin:
    ...
.LBB1:
    movl 16(%rdi), %edx
    testl %edx, %edx
je .LBB1
```

`-O0` and `-O1` were correct on the mutant; only `-O2` hoisted. This is the
Linux-kernel MMIO poll idiom; on real hardware it hangs.

---

## E2 — the existing subscript gate is blind to a hoist

Run against the **mutant** compiler from E1:

| instrument | result |
|---|---|
| `tests/regression/check_volatile_pointer_subscript.sh` | `ok` — exit 0 |
| `tests/regression/check_volatile_spin_loop.sh` (new) | `FAIL` — exit 1 |
| `scripts/check_volatile_destructuring.py` (new) | `FAIL` — exit 1 |

`check_volatile_pointer_subscript.sh` asserts `count_mem_movs(fn) >= 1`. A
hoist leaves exactly one access in the function, so the assertion holds on
miscompiled code. It is a correct instrument for *elimination* and an
incorrect one for *motion*.

This is why the fix for HIGH-1 could not be "wire the existing gate".

---

## E3 — why Clippy stayed green (the review's stated reason is wrong)

The review attributes the escape to rustc not linting refutable-pattern
bindings. Tested directly:

```
$ cat /tmp/lint2.rs      # refutable enum-struct pattern in an else-if-let
                         # chain; only `v` is dead
$ rustc --edition 2021 --crate-type lib -o /dev/null lint2.rs
warning: unused variable: `v`
warning: unused variable: `v`
$ rustc ... -D warnings
error: unused variable: `v`
```

rustc lints it. The actual cause is `src/lib.rs:2`:

```rust
#![allow(
    dead_code,
    unused_variables,   // <-- crate-wide; this is the hole
    unused_mut,
    unused_assignments,
    unused_imports,
    unreachable_code
)]
```

With the lint off, the dead binding produces no diagnostic and
`cargo clippy --all-targets --profile fastbuild -- -D warnings` (the CI
command) is structurally incapable of seeing the class. `scripts/check_volatile_destructuring.py`
is the replacement, scoped to the one field where silence is a miscompile.

---

## E4 — the static ratchet: 15-case self-test, 496 files, mutation-verified

`python3 scripts/check_volatile_destructuring.py --self-test` — every case is
a real shape from `src/passes`, not a synthetic caricature:

| case | expect |
|---|---|
| the LICM regression, verbatim shape | violation |
| explicit discard (`volatile: _`) | clean |
| binding with a real use | clean |
| match arm, not if-let | clean |
| a dropped **store** guard | violation |
| `volatile: false` construction | clean |
| mention inside a comment | violation |
| mention inside a string | violation |
| `volatile_x` is a different identifier | violation |
| `let .. else` whose use is in the **next statement** | clean |
| or-pattern binding into a tuple | clean |
| remapping `match`: the PATTERN | clean |
| remapping `match`: the CONSTRUCTION | clean |
| remapping `match` with the flag hardcoded to `false` | violation |
| unterminated body | violation (fail-closed) |

Cases 10–14 exist because a first cut of the detector reported **7 false
positives on real code** — `bit_idioms.rs`, `if_convert.rs`,
`loop_carried_forward.rs`, `slp_vectorizer.rs` (×2) and the `inline.rs` /
`outline_switch.rs` remapping arms. Every one bound `volatile` and used it;
none of the uses were where a "find the next `{`" arm-boundary scan looked.

Tree-wide result after the fixes: `ok (496 files)`.
Tree-wide result on the E1 mutant: the `licm.rs` violation, and nothing else.

The one real finding tree-wide was
`src/backend/generation.rs` — the codegen dispatcher bound `volatile` from
`Instruction::Load` and never used it. Adjudicated by hand (see the follow-up
doc §1.4): benign, because the backend emits one load per instruction and the
three sites that can rewrite an access all check the flag themselves. Changed
to `volatile: _` with the reasoning inline. No generated-code change.

---

## E5 — `loop_preheader` fires, but not on the shapes its docstring uses

Measured before writing the gate, because a gate written on inert shapes
asserts nothing.

| shape | default | `CCC_DISABLE_PASSES=loop_preheader` |
|---|---|---|
| `derived_sum` (guard at top) | identical | identical |
| `sqlite_shape` (`if (p==0) return 0;`) | identical | identical |
| `dowhile_sum` (guard outside) | `movl (%rdi),%edx` **before** `.LBB3` | `movl (%rdi),%r9d` **inside** `.LBB2` |

So the pass is inert on guard-at-top shapes — the frontend puts the guard in
the header, and the load then lives in a block that does not dominate every
loop block, so LICM's must-execute rule refuses it and the preheader would
unlock nothing. The `do`-while shape has its guard outside, so the header is
the body and the load dominates. That is the shape the gate uses.

`check_loop_preheader.sh` mutation, no rebuild needed:

```
$ CCC_DISABLE_PASSES=loop_preheader bash tests/regression/check_loop_preheader.sh
FAIL pass on:  dowhile_sum hoisted: 'dowhile_sum' has 1 memory access(es) in
     the loop body, want exactly 0
FAIL: the pass changed the emitted code for while_sum/guarded_sum/alloca_sum;
     the declined shapes must be byte-identical
```

---

## E6 — the two shape files the audit said did not exist

`tests/regression/loop_preheader/loop_preheader_shapes.c` and
`tests/regression/check_loop_preheader.sh` are the end-to-end coverage named
by `src/passes/loop_preheader.rs`'s own docstring. Neither existed in the PR.

The shapes live in a **subdirectory**, not the corpus root. This is the
BLOCKER-1 lesson applied to the file written to fix BLOCKER-1:
`run_regression.py` globs `tests/regression/*.c` non-recursively and builds
each standalone, so a second file that only makes sense next to the first one
would be compiled without its partner and fail with an undefined reference.

---

## E7 — MED-1, the vacuous PASS, and its two new known-answer cases

`reloc_oracle_agreement` with `oracles == []`:

* pre-fix: `floor = min(2, 0) = 0`, `all([]) is True` → `("PASS", "")`
* post-fix: `("FAIL", "no oracles configured to cross-check against: an empty
  agreement set is vacuously true, ...")`

Unreachable from the live call site (`bfd` is always seeded), which is exactly
why it needed a case. Suite: **30 → 32**.

```
  ok   an empty oracle set is a vacuous PASS and must FAIL
  ok   an empty oracle set fails even if notes claim agreement
```

---

## E8 — MED-2, the oracle cache, and the two cache self-tests

`_compiler_version(cid)` issues **one** request per compiler per process
(`GET /api/compilers`), records the CE version string *inside* the record, and
compares it on the way out. Mismatch is a **miss**: the sweep re-measures and
overwrites, and the eviction is reported in the sweep's summary line.

Deliberately not in the cache key — that needs a probe before every compile and
doubles requests against a rate-limited API. A record with no recorded version
predates the field and is still honoured.

Cache self-test **13 → 14**: strict text decode (well-formed round-trips;
undecodable bytes are a miss, not a U+FFFD hit; `load_lines` is `None`, not
`[""]`). Plus the `stats()` temp-file filter fix — temps are
`<stem>.tmp.<pid>.<tid>.json`, so `endswith(".tmp")` matched none of them.

---

## E9 — validation actually run

```
cargo build   --profile fastbuild -j 2                  clean
cargo fmt     --all -- --check                          clean
cargo clippy  --all-targets --profile fastbuild         clean (-D warnings)
                -- -D warnings
run_regression.py --filter minmax -j 2                  2 passed, 0 failed
check_minmax_reduction.sh                               PASS (4 contracts)
check_volatile_spin_loop.sh                             PASS  (FAIL on mutant)
check_volatile_pointer_subscript.sh                     PASS  (PASS on mutant)
check_volatile_destructuring.py                         ok 496 files (FAIL on mutant)
check_loop_preheader.sh                                 PASS  (FAIL when pass disabled)
test_godbolt_cache.py                                   PASS (14 checks)
test_reloc_oracle_verdict.py                            PASS (32 cases)
check_ci_gate_parity.py                                 PASS (101 commands)
```

Method: every "this gate can fail" claim is a **mutation**, not an argument.
The volatile guard deletion was reintroduced into a real compiler build and
measured, then reverted. The preheader gate was mutated with an environment
variable, for free.

# Follow-up 2026-11-04 — GCC 17 C-torture: real builtin semantics, ABI-correct lowering, measurable gains

Branch `arena/e0bf34d9-lccc`, PR **#1096**. Upstream `ms178/lccc` `main` advanced twice
during this session: `5a3b2087c…` then `5083e566a259d6a41d88ce817cfed9afadfddbc7`
(2026-11-04 14:16 +01:00). The final branch HEAD contains `5083e566a…` as an ancestor
(`git merge-base --is-ancestor origin/main HEAD` passes). The PR API's
`base.sha = 9bed91b…` is a *cached field*, not a missing rebase: `9bed91b…` is itself
an ancestor of the new main. Verified with `git ls-remote origin refs/heads/main`,
the commits API and `git merge-base --is-ancestor`.
Acceptance gate is **hosted GitHub CI only** ("Keine lokale CI!"). Local work was limited to
surgical unit/contract/runtime reproduction tests and the full torture *measurement* (not a
local CI gate).

## 1. What was actually wrong (not "the file did not compile")

The fresh GCC 17-git corpus (`c94b688a83fc84015c6fab8ab0ff142051598fc8`, 2016 compile /
1700 execute sources) measured 46 x86_64 and 47 i686 compile failures before this session's
compiler work. Root causes, established by direct compiler invocation and GCC 16.2 oracle runs:

* 13 compile-leg sources spell GNU builtins that LCCC did not register at all, producing
  `implicit declaration of function '__builtin_*'` (which is a hard error, not a warning) and
  leaving a literal `__builtin_*` symbol for the linker.
* `930529-1.c` was **not** an LCCC miscompile: its `dg-options` uses the Tcl brace-list
  spelling `{ "-fwrapv" }`, which the quote-only runner ignored, so the GCC 16.2 *reference*
  died with SIGILL (`ud2`) from signed-overflow UB. That was the single `reference-fail`
  in both execute legs.
* `__builtin_clear_padding` rejected GNU zero-length / VLA arrays as "incomplete type",
  although they are complete types with size 0 and GCC 16.2 accepts them
  (`pr98087.c`). LCCC's lowering already treats `size == 0` as a no-op.

## 2. Changes, and why each one is the right implementation

### 2.1 Shared builtin type table (`src/frontend/sema/builtins.rs`)

`gcc_compat_builtin_ctype(name)` is the single source of truth, called from
`ExprTypeChecker::builtin_return_ctype`, `ExprTypeChecker::infer_c2y_builtin_ctype`,
`Lowerer::builtin_return_type` and `Lowerer::get_c2y_builtin_ctype`. A drift between the
C-level type and the IR-level type is otherwise a silent miscompile (`sizeof(...)` says 4,
codegen returns 8 bytes). Four Rust unit tests pin registration, kind, CType and the
`size_t` parameter conversions; they run in hosted CI.

Registered libc aliases: `bcmp`, `bzero`, `bcopy`, `strdup`, `strndup`, `ilogb`, `modf`,
`modff`, `strncasecmp`. `bzero`/`bcopy`/`strndup` were added to
`libc_alias_param_types` so an `int` length is converted to pointer-width **unsigned**
`size_t` before the call — the exact defect class of `pr59229.c` (garbage high half of
`%rdx` on LP64). `bcmp`/`strncasecmp` already had their slots.

### 2.2 Five intrinsics (`src/ir/lowering/expr_builtins_gcc17.rs`)

| Builtin | Why a LibcAlias is wrong | Implemented |
|---|---|---|
| `__builtin_memcmp_eq` | GCC returns **0/1**, `memcmp` returns a signed byte difference. GCC 16.2 at `-O2` *and* `-O0` prints `1 0 -1` for unequal / equal / `memcmp`. Aliasing would change every equality test's false value. | one `memcmp` call (the vectorised libc implementation beats any inline byte loop) + `Cmp Ne` → `setne`; `size_t` conversion applied from the builtin prototype even with no declaration. |
| `__builtin_stack_save` / `__builtin_stack_restore` | Stack-pointer intrinsics, no libc symbol. | Direct IR `StackSave` / `StackRestore` — the same machinery used for VLA scope cleanup, so no duplicate stack accounting. |
| `__builtin_cexpi(x)` | No libc symbol; `sin`+`cos` would be two libcalls. | ONE glibc `sincos` call, then the complex value is built in LCCC's pointer-backed 16-byte representation (real +0, imag +8). No target complex-return ABI is involved. |
| `__builtin_cpow(x,y)` | libm uses the target complex ABI (x86-64: two SSE args each, xmm0/xmm1 return; i686: 16-byte stack aggregates + hidden sret). A LibcAlias would emit `IrType::Ptr` as the return carrier and read the result from `%rax` — a miscompile. | Decompose both operands into four adjacent F64 scalars (which *is* the x86-64 register layout and the i686 stack layout), call `cpow`; x86-64 uses `GetReturnF64Second`, i686 passes a 16-byte hidden sret buffer that **is** the pointer-backed complex result. |

Alternatives rejected: (a) aliasing `cpow` to `cexp(y*clog(x))` via scalar libm calls —
mathematically equivalent but deviates from glibc's special-case/branch-cut behaviour;
(b) computing `cexpi` with two separate `sin`/`cos` calls — double the call overhead;
(c) returning the raw `memcmp` value — wrong semantics; (d) teaching the assembler to
tolerate undefined local labels — would hide real lowering bugs (see §5).

### 2.3 DejaGnu option grammar (`tools/corpus/dejagnu.py`)

The shared parser now accepts quoted strings, Tcl brace lists and bare `-flag` tokens,
skips target-qualified directives, and refuses unterminated groups. Red-team findings that
were **caught and fixed before delivery**:

* the structural brace scan used the full file while the search used an 8192-byte window —
  an unterminated directive could be "closed" by a `}` in C code beyond the window. Both
  now use one bounded window;
* the first implementation dropped separated `--param NAME=N` values (4 real corpus
  sources: `pr86637-1.c`, `pr86637-3.c`, `pr88347.c`, `pr89235.c`) — quoted directives
  now preserve the exact legacy argv, brace lists keep flags plus `--param` values only;
* escaped quotes and unbalanced quotes are refused rather than guessed;
* negative `limit` is rejected instead of silently reinterpreted.

`scripts/check_dejagnu_options.py` proves corpus-wide that the new grammar is a strict
superset: **3716 sources, 657 with quoted directives, exactly one added flag
(`-fwrapv` in `930529-1.c`), zero regressions.**

### 2.4 Harness survivability

`scripts/arena_provision.py` restores pinned Rust 1.99.1 (sha256
`231fcae…`), GCC 16.2 (`4fa6af4f…`), bookworm ILP32 debs and the GCC 17-git corpus through
GitHub's reachable Git Blobs API after a harness wipe. It is idempotent, verifies every
SHA-256, rejects unsafe tar members, and asserts real LP64 **and** ILP32 compile+link+run.
A third harness wipe occurred during the red-team audit (it removed `/swapfile`, the whole
toolchain cache, `target/fastbuild`, `/tmp` probes and the snapshot, and reset the worktree
HEAD to `dfb902b` with the patchset as uncommitted changes). Recovery used `git fetch`,
`git diff --cached --binary origin/arena/e0bf34d9-lccc` to isolate ONLY the new work,
`git reset --hard` to the committed branch head and `git apply` — no work was lost and no
commit was rewritten. `scripts/ensure_swap.sh` enforces EXACTLY 4 GiB (4,294,967,296 bytes,
priority -2), recreating a wrong-sized swap and remaining idempotent on a second run;
`python3 scripts/arena_provision.py` then restored GCC 16.2.0, Clang 23.1.4, Rust 1.99.1,
19 pinned bookworm multilib debs and the GCC 17 corpus from the pinned Git Blobs.
`--verify-only` PASS.

## 3. Measured results

Toolchain: GCC 16.2.0 (`gcc-16`), Rust 1.99.1, corpus `c94b688a…`, `-O2`, jobs=2,
compile timeout 180 s, run timeout 10 s. Baseline = fresh pre-fix measurement this session;
after = full four-leg rerun after all compiler changes.

| Leg | Baseline | After | Delta |
|---|---|---|---|
| x86_64 execute (1700) | 1694 pass, **1 reference-fail**, 5 unsupported | **1695 pass, 0 fail**, 5 unsupported | +1 pass, −1 failure |
| x86_64 compile (2016) | 1954 pass, **46 compile-fail**, 15 ref-skip, 1 unsupported | **1966 pass, 34 compile-fail**, 15 ref-skip, 1 unsupported | **+12 pass, −12 compile-fail** |
| i686 execute (1700) | 1691 pass, **1 reference-fail**, 3 ref-skip, 5 unsupported | **1692 pass, 0 fail**, 3 ref-skip, 5 unsupported | +1 pass, −1 failure |
| i686 compile (2016) | 1946 pass, **47 compile-fail**, 22 ref-skip, 1 unsupported | **1958 pass, 35 compile-fail**, 22 ref-skip, 1 unsupported | **+12 pass, −12 compile-fail** |

**Final four-leg measurement after the label, builtin, harness and attribute fixes**
(`/home/user/.cache/lccc-torture/current/*.json`, 7432 cases):

| Leg | Pass | Candidate compile-fail | Reference-compile-skip | Unsupported | Candidate failures |
|---|---|---|---|---|---|
| x86_64 compile (2016) | **1970** | 30 | 15 | 1 | 0 execute failures |
| x86_64 execute (1700) | **1695** | — | — | 5 | **0** |
| i686 compile (2016) | **1961** | 32 | 22 | 1 | 0 execute failures |
| i686 execute (1700) | **1692** | — | 3 ref-skip | 5 | **0** |

`grep` over all four `*.failures.txt` logs found **zero** `label … used but not defined`,
`duplicate label` or `.LBB` diagnostics — the computed-goto class is fully eliminated.
The generated `.fails.txt` lists 30 x86_64 and 32 i686 compile failures; the i686-only
ones are `20041119-1.c` and `20111209-1.c`. These are the categorised, honestly-open
families in §5, not hidden regressions.

Total: **26 additional passing cases across the four legs; both execute legs now have zero
candidate failures.** The 12 fixed compile tests are `20021015-2.c`, `memtst.c`,
`pr51856.c`, `pr37669.c`, `string-large-1.c`, `pr110266.c`, `pr48641.c`, `20071117-1.c`,
`pr98087.c`, `pr108892.c`, `pr122588-1.c`, `pr54428.c` (12/12 pass on both targets);
`bcopy-1.c` was fixed afterwards and also passes on both targets (1/1 each), so the next
full run should read 33/34 compile-fails.

Validation actually executed (all PASS):

* `tests/regression/check_gcc17_builtin_semantics.sh` — runtime differential against
  GCC 16.2 on **x86_64 and i686**: `memcmp_eq` 0/1, `bcmp`, `bzero`, `strdup`,
  `strndup`, `strncasecmp`, `ilogb`, `modf`, `modff`, `cexpi` vs `cos`/`sin`,
  `cpow(1+2i,3) == -11-2i`, `stack_save`/`alloca`/`stack_restore`, plus `pr98087`
  zero-length `clear_padding` compilation. `PASS 0` on both targets.
* `cargo test --profile fastbuild --lib gcc17_torture_builtin_tests` — 4/4 OK.
* `python3 -m unittest discover -s tests/corpus -p test_dejagnu_options.py` — 14/14 OK.
* `python3 -m unittest discover -s tests/corpus -p test_x86_torture_contracts.py` — 35/35 OK.
* `scripts/check_dejagnu_options.py` — corpus-wide strict-superset proof, PASS.
* `scripts/check_ci_gate_parity.py` — PASS (529 commands, 27 contracts);
  `scripts/check_ci_workflow_shell.py` — PASS; `bash -n` on both shell scripts — PASS.
* Targeted `930529-1.c` execute with GCC 16.2 — pass on x86_64 and i686.

No hardware PMU is available on this virtual Xeon (2 vCPUs), so **no** hardware-counter
or Raptor Lake claims are made. Generated-code performance arguments in §2 are about
instruction/call counts (`sincos` = 1 libcall vs 2; `setne` = 1 instruction; no extra
stack traffic for stack_save/restore), not measured cycle counts.

## 4. CI wiring

The new runtime gate is executed by BOTH mirrors: `gate "gcc17-builtin-semantics" fast`
in `scripts/ci_local.sh` and an explicit `run:` step in `.github/workflows/ci.yml`.
`check_ci_gate_parity.py` is what forced the hosted step; without it CI would have been
red even though the compiler was correct. Commit `6c664f5` (parser-only) had **all four
hosted workflows green**; later commits are validated by fresh runs (final status is
reported in the delivery message, not guessed here).

## 5. Honest remaining gaps (34 x86_64 / 35 i686 compile-fails, categorised)

These are **not** hidden and **not** claimed fixed:

1. **4 parser-depth limits** (`limits-caselabels.c`, `limits-exprparen.c`,
   `limits-pointer.c`, `limits-structnest.c`): LCCC's recursive-descent hard limits
   (8192 blocks / 32768 expressions / 4096 pointers / 2048 types) are below GCC's.
   Raising them blindly risks host stack overflow; the correct fix is an iterative
   parser path for those constructs.
2. **RESOLVED: vector-type attribute scope.** `vector-1/2/3.c` were not three
   semantics bugs but ONE parser state leak: `ParsedDeclAttrs::save_flags()` packs only
   booleans, so `parsing_vector_size`, `parsing_vector_size_expr` and
   `parsing_ext_vector_nelem` survived into the next declaration parsed. A function
   RETURN TYPE's attribute was also consumed by its parameter list (declarators are parsed
   before `parse_function_def` builds the return type). Fixed at the source: save/restore
   the three Options around every compound statement and every parameter list, apply the
   attribute to function/nested-function return types and to parameters. All three torture
   files now compile. **RESOLVED in part: `20001116-1.c`** (same-string-literal pointer
   difference) — sema accepts the GCC 16.2 rule (identical literal text only), lowering
   folds the byte difference, with pointer/pointer-width-integer cast scaling; runtime
   values `1 2 1 1` match GCC 16.2 on x86_64 AND i686. Clang 23.1.4 rejects even the
   same-text form; this is a documented GNU divergence. **Still open:**
   `20010113-1.c`, `20050516-1.c` (address-of-member through a cast), `pr48517.c`
   (static array from a compound literal), plus asm constraints, parser-depth `limits-*`
   and deliberate per-vectorizer flags. `941014-4.c` is already fixed by the DejaGnu
   `dg-options "-std=gnu89"` plumbing (`extract_option_flags` returns `('-std=gnu89',)`).
   **All four are now RESOLVED** (see §7.4): `vector-2.c` (vector struct
   member), `20010113-1.c` and `20050516-1.c` (address of a member through
   nested/absolute pointer chains), `pr48517.c` (unsized static array from a
   whole-array compound literal). Only the parser-depth `limits-*`, inline-asm
   constraints and deliberately unsupported per-vectorizer flags remain.
3. **4 inline-asm "impossible constraint"** (`20000922-1.c`, `asmgoto-2.c`,
   `pr27528.c`, `pr35318.c`): constraint alphabet coverage.
4. **RESOLVED: 2 internal assembler errors** (`.LBB1`/`.LBB2` in `pr17913.c`,
   `pr84136.c`). Three distinct defects, three fixes: (a) the x86 identical-block
   peephole merged an address-taken `.LBB` — now refused via `collect_lbb_tokens` /
   `address_taken_labels`; (b) a `&&label` in a constant-folded `?:` arm allocated a
   label ID with no basic block — `materialize_undefined_user_labels` appends a
   deterministic landing pad; (c) the backfill alone silently accepted genuinely
   undefined labels, so a zero-copy GNU/C label-namespace validator was added in sema.
   `tests/regression/check_computed_goto_labels.sh` proves runtime dispatch on both ABIs
   at -O0/-O2, zero dangling `.LBB` references (48 defined/19 referenced at -O0,
   23/14 at -O2), 7 negative diagnostics and 4 valid GNU shapes per ABI. A 34-probe
   GCC 16.2 differential agrees 34/34; GCC 16.2, Clang 23.1.4 and LCCC all reject the
   five invalid label programs.
5. **4 deliberately unsupported GCC flags**: `-ftree-loop-vectorize`,
   `-ftree-slp-vectorize` (`pr86636.c`, `pr86637-1.c`, `pr86637-3.c`, `pr99787-1.c`)
   and `-fopenmp` (`pr89235.c`). `src/driver/cli.rs` deliberately rejects per-vectorizer
   control because LCCC's `-O2` pipeline runs `vectorize`/`slp` passes as a group and
   claiming per-pass support would be dishonest. Accepting them as silent no-ops or
   disabling *all* vectorizers for a single `-fno-tree-slp-vectorize` would either lie
   or throw away performance; precise per-pass plumbing is the correct next step.
6. **Remaining parse/declaration errors** (`20101216-1.c`, `930525-1.c`, `991213-2.c`,
   `pr115143-2/3.c`, `pr72802.c`, `pr85401.c`, `pr90275.c`, `pr96998.c`): mostly
   GNU statement-expression / zero-length-array / nested-declaration corners.
7. `pr67106.c` is a deliberate data-model rejection (`-m32`-class mismatch) — correct
   behaviour, not a defect.

## 6. Delivery artefacts

* `/home/user/ms178-1.patch` — regenerated after each validated fix with sane
  `S0x-*` names; applies clean against `5a3b2087c…`. The snapshot ledger carries
  `ci_gate=UNGATED` because the user forbade local CI as the acceptance gate; the
  authoritative gate is hosted GitHub CI on PR #1096. Exact byte counts and SHA-256
  values live in `/home/user/artifacts/` (a harness wipe erases them, so they are not
  duplicated as stale constants in this document).
* `/home/user/artifacts/` — timestamped patch series, source tar, session bundle,
  snapshot ledger and `.base_ref` anchor.
* Reports: `/home/user/.cache/lccc-torture/current/{arch}-{mode}.json` and
  `.failures.txt` (wipe-volatile; the numbers are recorded below).

## 7. Red-team continuation (2026-11-04, hosted CI failure driven)

Hosted run `41816433250` at `e326d441d` was **red**: `Fast local contracts (shard b)`
failed at step 21 and the aggregate `Test Suite` step failed with it. GitHub's
`results-receiver.actions.githubusercontent.com` is unreachable from this sandbox, so the
log was recovered through the **Checks API annotations**
(`/check-runs/126529773680/annotations`), which named the exact failure:

```
FAIL: test_no_unconditional_data_filter_anywhere_in_the_tree
AssertionError: Lists differ: ['scripts/arena_provision.py:164'] != []
```

That was a regression from the earlier Clang-archive extraction fix: a THIRD
`extractall(filter="data")` site in `safe_extract`, outside the two guarded helpers the
repository's safety contract permits. Fixed at the design level, not by adding an
allowlist entry:

* the helper is now `extract_untrusted`, the repository's canonical guarded-extractor
  name, with unconditional member validation BEFORE any write (traversal, absolute /
  escaping symlink and hardlink, device node, FIFO);
* the capability probe is now `extractall_takes_filter()` using
  `inspect.signature(tarfile.TarFile.extractall)` — the exact API being called —
  instead of `hasattr(tarfile, "data_filter")`, which tests a DIFFERENT object. It
  fails closed on `TypeError`/`ValueError` and detects a `backports.tarfile` patch
  without hardcoding a Python version tuple;
* `tests/corpus/test_arena_provision_extraction.py` adds 11 tests: probe parity and
  fail-closed, `filter=` present ONLY when supported, benign extraction on legacy
  Python, 7 unsafe-member refusals with a proof that nothing was written, safe symlink /
  hardlink acceptance, the native filter as a second layer, and AST wiring invariants.

A second, unrelated defect was then uncovered by the NEW i686 regression case: a GNU
nested function returning a 16-byte vector panicked with
`wide i686 cast reached the scalar cast emitter`. Root cause traced through
`LCCC_DUMP_IR`: the nested path lowered the return to
`Cast { from_ty: Ptr, to_ty: I128 }` — the ADDRESS bits, not the object — while the
already-correct top-level path emits `Load { ty: I128 }`. `emit_cast_default`'s
integer→i128 branch then called `emit_cast_instrs(Ptr, U64)`; on i686 `U64` is a
register pair, which the scalar emitter correctly refuses with an assert. Fixed in
`src/ir/lowering/stmt_return.rs` so both ABIs and both function kinds share the
`Load { ty: I128 }` representation. After the fix the nested IR is exactly
`Alloca; Load I128; Return`, and GCC 16.2 / LCCC x86_64 / LCCC i686 all return 0.

### 7.1 New regression gates and measured results

`tests/regression/check_type_attr_and_static_init.sh` (final measured 3.569 s /
2.101 s / 3.405 s; worst kept as `3.57` in the lane plan) runs on x86_64 and
i686 at -O0 and -O2:

* runtime vector ABI (`v4si` add, direct vector-size parameter and return, sizeof 16),
  with GCC 16.2 compiling the SAME source as the value oracle: `PASS 0` in all 8 runs;
* folded static address constants `&"Foobar"[1]-&"Foobar"[0]`,
  `[3]-[1]`, `(int*)`-scaled, `(long)`-byte-difference and a block-scope `static`:
  `1 2 1 1 2`, identical to GCC 16.2 on both ABIs;
* a negative that must produce EXACTLY ONE diagnostic (at `return t`, not at the local
  `float t`) plus GCC 16.2 rejection; a different-literal difference must be rejected;
* 6 positive attribute-scope shapes including prototypes and GNU nested functions;
* a **runtime** nested-vector-return differential at -O0/-O2 against GCC 16.2 on both
  ABIs;
* `runtime2.c`, a GCC 16.2 value-oracle runtime proof of the four §7.4 families
  at -O0/-O2 on both ABIs. Final line:
  `type-attribute scope and static-init address-constant contract: PASS`.

Eight Rust AST unit tests in `src/frontend/parser/declarations.rs` pin the parser state
machine itself (return type vs first local, parameter list not stealing the return
attribute, plain parameter, no leak into the next global, nested compound statement,
nested function, vector struct member applied to that member only, no leak out of
a record body): 8/8 pass.

### 7.2 Full local validation after the fixes (not the acceptance gate)

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --profile fastbuild --locked -j 2 --all-targets -- -D warnings` | rc0, zero warnings |
| `python3 scripts/check_clippy_policy.py` | PASS, 76 lints, 0 unknown |
| `cargo test --profile fastbuild --lib -- -q` | **7931 passed, 0 failed, 10 ignored** (14.80 s, after integrating latest main) |
| `python3 -m unittest discover -s tests/corpus -p 'test_*.py'` (the exact failed CI command) | **1195 tests, 0 failures, 5 skipped** (194.2 s, after integrating latest main) |
| `tests/regression/check_type_attr_and_static_init.sh` | PASS, both ABIs, -O0/-O2 |
| `tests/regression/check_computed_goto_labels.sh` | PASS, both ABIs, -O0/-O2 |
| `check_ci_lane_plan.py` (+ `--self-test`) | PASS (2935 s, 18 rules / 25 fixtures, after integrating latest main) |
| `check_ci_gate_parity.py` | PASS (532 commands, 27 contracts) |
| `check_ci_workflow_shell.py` | PASS (13 workflows) |
| `python3 scripts/arena_provision.py --verify-only` | PASS, LP64 + ILP32 compile/link/run |

The two local-only failures seen before the environment was restored were NOT code
defects: `backports.tarfile` missing after the wipe (installed from PyPI) and
`gcc-16` absent from `PATH` so `test_nostdinc_contract` fell back to Debian GCC 12.2.
Hosted CI provisions both.

### 7.4 Four more GCC 17 compile-leg families, found by running the REAL corpus

The 10-file targeted compile matrix (`--filter '^(vector-[123]|20001116-1|
pr17913|pr84136|20010113-1|20050516-1|pr48517|941014-4)\.c$'`, both ABIs)
went 6/10 -> 7/10 -> **10/10 pass**. A simplified hand-written probe had hidden
one defect, which is why the real corpus is authoritative:

| Family | Root cause | Fix |
|---|---|---|
| `vector-2.c` | `StructFieldDecl` has no `vector_size` slot, so a `__attribute__((vector_size(16))) float mVec;` MEMBER silently became a plain `float` and the pending `Option` leaked to the next member. `vector-1.c` only takes `&y.mVec` and so never type-checked the member. | `parse_struct_fields` now saves/restores the three vector `Option`s around the record body, clears them per member, and applies `apply_pending_vector_attr` to the member's BASE spec before `fold_simple_derived` adds pointer/array layers. A deferred `vector_size(EXPR)` is carried on a new `StructFieldDecl::vector_size_expr` and resolved in sema's `field_ctype` with `wrap_field_spec_base`, so it cannot leak or wrap a folded pointer type. 2 new AST unit tests pin it. |
| `20010113-1.c`, `20050516-1.c` | `resolve_pointer_member_access_addr` took the field offset from `globals[name].struct_layout` -- the layout of the *whole global*, not of the pointer's pointee. `&((&(v.p))->y)` addresses `y` inside the nested `p`; `&((struct T *)0x4000)->t` has no global at all. | The field offset now comes from `get_expr_ctype(base)`'s pointee layout (legacy global-layout path kept as fallback). An absolute integer address cast to a pointer is emitted as a width-correct `GlobalInit::Scalar` (`.long 16384` on i686, `.quad 16384` on x86_64), never a fake relocation. Runtime values `4` and `0x4000` match GCC 16.2 on both ABIs. |
| `pr48517.c` | `static const unsigned short a[] = (const unsigned short[]){...}`: sema's `non_constant_leaves_for` required the literal's type to EQUAL the unsized target; lowering's `array_initializer_payload` required a sized target; `fixup_unsized_array` could not read a bound from an expression initializer, so `sizeof(a)` stayed at the 256-element incomplete-array placeholder (512 bytes instead of 4). | Accept a literal whose bound supplies an unsized target's bound; allow `CType::Array(..)` in `array_initializer_payload` (`expr_initializes_array` already compares element types and known bounds); compute `unsized_array_len` from the literal's braced payload. `sizeof(a)` and every element value now match GCC 16.2 on x86_64 and i686 at -O0/-O2. |

All four are pinned by `runtime2.c` inside
`tests/regression/check_type_attr_and_static_init.sh`, which compiles the SAME
source with GCC 16.2 as a value oracle on both ABIs at both optimization levels.
The gate grew from 0.92 s to 3.57 s (3 timings 3.569/2.101/3.405 s, worst kept);
`.github/ci-lane-plan.json` frontend-a is now `measured_s 3.57`,
`predicted_gate_seconds 356.96`, and `check_ci_lane_plan.py` PASS (2934 s of
gate cost).

### 7.3 Three-compiler comparison

GCC 16.2.0 and Clang 23.1.4 ran locally; ICX 2026.2 is not installable in this sandbox,
but the hosted job **"Authenticated Intel ICX 2026.2 integration" passed (1m47s)** on
this branch, and the workflow's ICX parity steps are the third oracle. Measured
agreements: vector parameter/return `sizeof` = 16 (GCC/Clang/LCCC); vector add
`11 22 33 44` (all three); prototypes + nested-function vector returns (GCC/LCCC;
Clang has no nested functions); invalid vector return rejected by all three with the
same class of diagnostic; same-literal static difference (GCC/LCCC accept and agree on
values, Clang rejects); five invalid label programs rejected by all three. No hardware
PMU is available, so all performance statements remain instruction/call-count arguments.

## 8. Integration with the latest `main` (2026-11-04, final)

`main` advanced to `5083e566a…` while this branch was under hosted validation.
Evidence, not assertion:

```
git merge-tree --write-tree origin/main HEAD  -> af24f7da218af5dd5b23b51dca4105857085dcd6 (rc=0)
GitHub pull_request synthetic merge 98f2a984b:
    parents = [5083e566a (main), 2da79402b (this branch)]
    tree    = af24f7da218af5dd5b23b51dca4105857085dcd6   # IDENTICAL
```

So the hosted `pull_request` runs already validate exactly the tree obtained by
integrating latest main. A literal 31-commit `git rebase origin/main` was attempted
and aborted: it conflicts at 21/31 in `.github/ci-lane-plan.json`, whose structure
`main` rewrote (858 vs ~1800 lines). Replaying would have required a force-push of a
hand-merged tree DIFFERENT from the one CI validated. The chosen integration is a
merge commit that keeps all 31 per-change commits, makes `5083e566a…` an ancestor,
leaves the tree byte-identical to `af24f7da…`, and is a plain fast-forward push.

Post-merge local gates (all PASS, not the acceptance gate): fastbuild 0,
`cargo fmt --check` 0, `cargo clippy --all-targets -D warnings` 0, clippy policy
(76 lints), 7931 unit tests / 0 failed, 1195 corpus unittests / 0 failures,
lane plan 2935 s + 18/25 self-test, gate parity 534/27, workflow shell 14 files,
shard partition a=85 / b=116, toolchain bundle OK, all three regression gates on
both ABIs at -O0/-O2, and the 10/10 targeted GCC 17 compile matrix on both ABIs.

Final four-leg partial measurement at `-O2` with GCC 16.2 as oracle and jobs=2:
x86_64 compile (2016) = **1977 pass / 23 compile-fail / 15 reference-compile-skip /
1 unsupported / 0 reference-fail**, up from the historical 1970/30 baseline
(+7 pass, -7 fail). The remaining 23 are the honestly open families in §5
(parser-depth `limits-*`, inline-asm constraints, deliberate per-vectorizer flags,
and remaining GNU declaration corners).

## 9. Follow-up PR #1101 (branch `arena/1a41faea-lccc`, 2026-11-04)

New PR from merged `main` `49489a5d51d2dca602c171f686cb358058965a45`. Every
number below is a fresh measurement on this branch with the fastbuild preset,
a 4 GiB swap file, `gcc-16 (GCC) 16.2.0` as oracle and Clang 23.1.4 /
hosted ICX 2026.2 as cross-checks. Baseline is the fresh merged-main
compile matrix in `/home/user/.cache/lccc-torture/baseline/`.

### 9.1 Measured compile-leg movement

| leg | baseline | PR #1101 | delta |
|---|---|---|---|
| x86_64 `-O2` compile | 1977 pass / 23 fail / 15 ref-skip / 1 unsupported | **1994 pass / 6 fail** / 15 ref-skip / 1 unsupported (S09) | **+17 pass, −17 fail** |
| i686 `-O2` compile | 1968 pass / 25 fail / 22 ref-skip / 1 unsupported | **1987 pass / 6 fail** / 22 ref-skip / 1 unsupported (S09) | **+19 pass, −19 fail** |

Two further i686-only defects were found and fixed after the S03 measurement
(`20041119-1.c`, `20111209-1.c`). The next full matrix (S04, after both fixes)
is **x86_64 1993 pass / 7 compile-fail / 15 ref-skip / 1 unsupported** and
**i686 1986 pass / 7 compile-fail / 22 ref-skip / 1 unsupported** — both legs
now have the *same* seven remaining non-passes, listed with their root causes
in §9.4. S08 (§9.5) then fixed `991213-2.c` on BOTH legs, leaving six
compile-fails per leg and no regression anywhere in the corpus. **Matrix
hygiene (red-team of the evidence itself):** S05 was
discarded because its harness silently fell back to `/usr/bin/gcc` (Debian
12.2.0) instead of the pinned GCC 16.2 oracle — 12 reference ICEs and one
60 s timeout were compiler bugs of the WRONG oracle, not LCCC regressions.
S06 was deliberately invalidated by the harness itself
(`run inputs changed mid-flight: tools`) because a fastbuild relink happened
while the matrix was running. The clean, unmodified-binary final matrix is
S09 (§9.5). Both discarded runs are kept as evidence that no number here is
accepted without checking the oracle identity and the run's `tools` digest.

### 9.2 Fixed families (change-by-change)

1. **Inline-asm constraints (4 cases).** `asm_validate::check_constraint`
   modelled a constraint as a comma-free set of letters. It now implements the
   measured GCC 16.2 grammar: comma-separated ALTERNATIVES (accept if ANY is
   satisfiable), cost/selection modifiers `? ! ^ $ *`, `#` comment tails with
   punctuation validation, operand-count consistency, matching-digit range
   checks, and a shared grammar in `common::asm_constraints` plus both x86 and
   i686 `classify_constraint`. `s`/`n`/string-literal constness rules were
   measured, not guessed. An old test encoding a **gcc 14.2** measurement
   (`"n"(symbol)` accepted) was corrected against 16.2.
2. **Per-vectorizer flags (4 cases) + `-fopenmp` (1 case).** GCC has three
   independent tri-state switches; explicit per-family flags override the
   master in either order and last-wins within a family (measured with
   `-fopt-info-vec-optimized`). `passes::VectorizePolicy` carries them from
   CLI through `Driver` to `run_passes`. `-fopenmp` selects OpenMP's
   single-thread SERIAL execution model (all `#pragma omp` directives are
   no-ops, which is a conforming OpenMP execution); gcc 16.2 emits ZERO
   `GOMP_*` calls for `pr89235.c`'s `declare simd` shape. The observable
   effect is `_OPENMP 202111` (measured `gcc-16 -fopenmp -dM -E`); it is
   `Class::Implemented` in the GP-22 option-effect ledger, not an inert noop.
   Runtime entry points are resolved by the linker, never fabricated.
3. **GNU declaration corners (6 cases, two root causes).** (a) Implicit-int
   file-scope declarations under `-fpermissive` now emit ONE correctly
   classified `WarningKind::ImplicitInt` warning and yield `int`; the
   diagnostic engine owns the C23 default-error promotion, so default-mode
   parity with GCC is preserved (measured: both reject with 3 errors).
   `struct S { int x };` is a GCC pedwarn, not a parse error. (b) C11 6.9.2p2
   tentative definitions of a record completed LATER in the TU are now
   resolved at the END of the TU in `sema::analysis`; a still-incomplete
   object at end of TU remains an error (measured against GCC).
4. **`limits-pointer.c`.** `POINTER_DEPTH_BUDGET` was 4096, calibrated for a
   hypothetical 8 MB stack, while the compiler thread actually has 64 MB.
   Re-measured at 10k/20k/32k/65k stars (0.010/0.014/0.023/0.039 s, all rc0)
   and raised to 65,536, still >6x below the ~400k-star clone/drop stack
   overflow point. 80k stars produce exactly one clean budget diagnostic.
5. **Two pre-existing i686 defects.** `{esi}` explicit-register constraints
   were misclassified as "immediate-only" (letters e/s/i) by
   `constraint_is_immediate_only`; and `__attribute__((mode(SI)))` on a
   POINTER typedef rewrote `char *` into `int *`. GCC 16.2 accepts `mode(SI)`
   on `-m32` (SI is the pointer mode) and rejects it on x86-64 with
   `invalid pointer mode 'SI'`; `mode(DI)` is the 64-bit spelling. LCCC now
   matches both directions.
6. **Red-team: the deferred-incomplete fix had a semantic hole.** The first
   version deferred *every* file-scope incomplete object, so
   `struct c x = {0}; struct c { int e; };` was silently ACCEPTED, while gcc
   16.2 refuses (`variable 'x' has initializer but incomplete type`). C11
   6.9.2p2: an INITIALIZED declaration is a definition, not a tentative
   definition; only the latter may be completed later. `sema::analysis` now
   distinguishes the three cases (initializer / `extern` / tentative) and
   uses gcc's exact wording. Eight shapes (file scope, block scope, `extern`,
   still-incomplete) are pinned in the gate; all eight match gcc 16.2's
   verdict AND diagnostic.
7. **Red-team: four more inline-asm grammar holes.** A 64-spelling
   accept/reject differential against gcc 16.2 found and fixed: (a) an
   unmodelled letter is an *impossible ALTERNATIVE*, not a validity error
   (`"z,r"` / `"r,z"` compile, `"z"` alone does not); (b) a bare `Y`/`C`
   class prefix without its following letter is impossible, and a `#`-only
   constraint is `inconsistent operand constraints in an 'asm'`; (c) `%` on
   the LAST operand is `'%' constraint used with last operand`; (d) a
   matching digit after a class pair (`"Yz0"`) must be range-checked too.
   Red-teaming the fix itself then caught an OVER-rejection: the first
   implementation aborted an alternative on the first unmodelled letter,
   wrongly refusing `rz`, `zr`, `zri`, `rzi`, `Yzz`. gcc 16.2 accepts all
   five because a class letter is still present in the SAME alternative;
   the loop now keeps scanning. All **64 spellings** agree with gcc 16.2 on
   accept/reject (0 divergences); verbatim diagnostics are pinned in 42
   Rust unit tests.
8. **Red-team: `mode(M)` was implemented three times, divergently.**
   `declarations.rs` (file + block scope) and `nested_functions.rs` each had
   their own rule; the nested-function path still did an unconditional
   `mk.apply(type_spec)`. One `Parser::apply_mode_attribute` now serves all
   three, with the measured gcc 16.2 rule: FUNCTION/ARRAY declarator →
   `mode 'M' applied to inappropriate type`; POINTER → target-mode validation
   with the pointee untouched; scalar → base-type transform. A 10-shape
   differential on BOTH ABIs is pinned in the gate (§9.4 lists the
   specifier-position shapes that remain open).

### 9.3 New regression coverage (all wired into BOTH CI mirrors)

`check_gcc_asm_constraints.sh` (21 checks, incl. the 64-spelling gcc 16.2
accept/reject differential), `check_vectorizer_openmp_policy.sh` (31),
`check_gnu_declaration_corners.sh` (22, incl. 8 C11 6.9.2p2 shapes and the
2×10-shape two-ABI `mode(M)` differential), `check_parser_depth_limits.sh`
(4). They cover both ABIs, exact GCC diagnostics, end-to-end runtime
execution, artifact-level policy effects and the still-correct refusals.
CI/local gate parity PASS, workflow shell audit PASS, lane plan PASS
(frontend-a 360.43 s / 59 steps after re-measuring worst-of-3 gate costs),
option-effect ledger PASS, pipefail/SIGPIPE detector PASS. The swallowed
`|| true` in the GNU gate was removed; the gcc oracle's exit status is now
asserted.

### 9.4 Honest remaining gaps (NOT claimed fixed)

| case | root cause | measured oracle |
|---|---|---|
| `limits-caselabels.c` | 100,000 `case` labels are parsed/lowered as a 100k-deep RECURSIVE chain; `SwitchFrame::overlaps` is also O(n²) | gcc 16.2 0.331 s, clang 0.225 s, lccc clean budget error in 0.101 s |
| `limits-exprparen.c` | 10,000 nested parentheses need ~65k expression frames (budget 32768); the 64 MB thread stack is too small for the recursive descent | gcc 16.2 0.026 s; clang 23.1.4 **segfaults** (rc −11) |
| `limits-structnest.c` | 10,000 nested anonymous record definitions; quadratic `TypeSpecifier`/`Vec<StructFieldDecl>` clone cascades (callgrind: 812,222,167 Ir at depth 1000, dominated by malloc/free/clone) | gcc 16.2 1.874 s; clang 23.1.4 **segfaults** (rc −11) |
| `pr115143-2.c`, `pr115143-3.c` | `-fgimple` / `__GIMPLE` — GCC's internal GIMPLE front end, not ISO/GNU C | gcc 16.2 accepts; deliberately out of scope |
| `pr67106.c` | `-fpack-struct` data-model request | **deliberate, correct** rejection: ignoring it would silently change object layout (see `src/driver`) |

`991213-2.c` was REMOVED from this table after S08: the K&R parenthesised-name
declarator `void (* const (funcp)(time_t))` now parses and is typed correctly
on both ABIs (§9.7). It is no longer an open gap.

**`ovf.c` diagnostic parity:** no exact `ovf.c` exists in this repository, in
the pinned GCC 17 testsuite, or in the GCC mirror (checked with `find`,
`gh api contents`, `gh api search/code`). The only matches are fuzzy:
`gcc.dg/tree-ssa/minus-ovf.c` (an optimisation dump test that all three
compilers compile with ZERO diagnostics) and `aarch64/fmovf.c` /
`vect-fmovf.c`. The repo's one documented *overflow diagnostic* gap is
`engineering/FOLLOWUP-2026-09-27-S11-decimal-f33-redteam.md` §Open: "lccc
`-Woverflow` diagnostic parity (GCC warns on peUnder/peOver-style folds;
lccc is silent)" for decimal-float constant folding. That is a separate
item and is NOT claimed fixed here — the exact file the requester meant must
be identified before any claim of parity can be made.

**Parser-depth red-team result (measured, not guessed):** simply raising all
budgets and the stack is NOT a valid fix. With `PARSER_FRAME_BUDGET=262144`,
`BLOCK_FRAME_BUDGET=262144`, `TYPE_FRAME_BUDGET=65536` and
`POINTER_DEPTH_BUDGET=65536`: `limits-caselabels.c` aborted (rc −6) at
64/128/256 MB and timed out >120 s at 512/1024 MB; `limits-exprparen.c`
needed ≥256 MB; `limits-structnest.c` stayed 24.6–25.4 s at every stack
size. Deep-anonymous-struct generator at 1 GB: depth 500/1000/2000/4000/
8000/10000 → 0.034/0.103/0.473/3.148/15.705/25.598 s (quadratic). The
correct fix is iterative/non-recursive parsing and lowering plus indexed
duplicate detection and elimination of the type-field clone cascades — a
substantially larger change than a budget bump, and it is left OPEN with
evidence rather than papered over.

**Remaining `mode(M)` gaps (measured, pre-existing, NOT claimed fixed):**
`__attribute__((mode(M)))` in SPECIFIER position is applied to the base type
specifier in `parser::types::parse_type_specifier` before the declarator is
known. gcc 16.2 instead applies it to the completed declaration, so
`__attribute__((mode(SI))) int f(void)` (function),
`__attribute__((mode(SI))) int x[3]` (array) and
`__attribute__((mode(SI))) int *p` / `void f(char *p __attribute__((mode(DI))))`
(pointer mode on a parameter or struct field) still diverge on x86-64 / i686
respectively. Both other spellings (declarator- and post-declarator-position)
are covered by the central rule. Fixing this requires deferring the
specifier-level mode until the declarator is parsed across typedefs, fields,
parameters and casts — a parser-wide refactor with regression risk, deliberately
left OPEN with the 4 x86-64 / 5 i686 measured divergences recorded rather than
papering over them. No GCC 17 torture case uses these shapes.

**Hosted CI red → repaired (data-driven):** run `41882988078` on the S02
revision had exactly two failures: "Pipefail/SIGPIPE detector" (job
`126762142450`) and "Verify toolchain bundle and shard-partition contracts
(mock only)" (job `126762142144`). The first was caused by seven
`printf ... | grep -q` producer/SIGPIPE sites in the two new shell gates;
all were rewritten to `grep -q ... <<<"$var"`. The second was caused by four
new CI steps missing from `.github/ci-lane-plan.json`; the lane plan now lists
all four with measured costs. `check_pipefail_sigpipe.py` passes across 344
CI-run scripts and `check_ci_lane_plan.py` passes. `cargo fmt --all` is clean
(the earlier "FMT_CHECK_RC=0" was a `| head` pipeline artifact masking two
real diffs).

**Diagnostic-parity gaps for INVALID programs (tracked):** GCC 16.2 rejects
`"m"(5)`, `"s"(x_param)`, `"i"(x_param)` and `"n"(symbol)` in dead /
empty-template asm at `-O2`; lccc's deferred immediate validation currently
accepts them because the post-optimisation check only sees SURVIVING
`InlineAsm` instructions. No VALID program is miscompiled by this.

**Toolchain honesty:** GCC 16.2.0 and Clang 23.1.4 ran locally on every
measurement; hosted CI's "Authenticated Intel ICX 2026.2 integration" job is
green on this branch and is the third oracle. No PMU/`perf` is available on
this host, so NO cycle claims are made; performance evidence is wall-clock,
callgrind instruction counts and artifact comparisons.

### 9.5 Final clean matrix (S09)

Re-run with the pinned GCC 16.2 oracle, an unchanged fastbuild binary for the
whole run, and no concurrent cargo work. Filled in below from the JSON
`tools`/`complete`/`fatal` fields; a run with `fatal` set is NEVER quoted.

| leg | S07 (pre-K&R fix) | S08 (K&R fix) / S09 (final) |
|---|---|---|
| x86_64 `-O2` compile | 1993 pass / 7 fail / 15 ref-skip / 1 unsupported | **1994 pass / 6 fail** / 15 ref-skip / 1 unsupported |
| i686 `-O2` compile | 1986 pass / 7 fail / 22 ref-skip / 1 unsupported | **1987 pass / 6 fail** / 22 ref-skip / 1 unsupported |

Both S08 and S09 JSONs report `complete=True`, `fatal=None`, the pinned
`/home/user/.cache/lccc-toolchains/gcc-16.2/gcc-16/bin/gcc-16` oracle
(sha256 `67ed99639f746b8e…`) and a `tools` digest that is unchanged
WITHIN each run (the lccc hash naturally differs between S08 and S09). The S08 runner
logged the same lccc sha256 (`98ab3736ef1bf37b…` x86_64,
`bafc4e1c515af23a…` i686) before and after its two legs; the S09 runner
logged `61819b0444a8fd831…` / `fa1875392aa358c1…` unchanged before and
after ITS two legs, so no
relink occurred mid-flight; `dependency_digest` is byte-identical to S07
(`071a13826a907b4a…`), i.e. same corpus, flags and reference contract.
`lccc_head` records `8e0d1b1bb` because these binaries were built from the
worktree that was committed immediately afterwards; the binary hash, not the
commit label, is the identity that matters and it is pinned above.

**S07 -> S08 per-case diff: exactly ONE changed status on each leg,
`991213-2.c` `compile-fail` -> `pass`, and ZERO other changes across 4032
case-runs** (2 x 2016). The six remaining compile-fails on both legs are
`limits-caselabels.c`, `limits-exprparen.c`, `limits-structnest.c`,
`pr115143-2.c`, `pr115143-3.c` and `pr67106.c`, itemised in §9.4; the single
`unsupported` case is `limits-fnargs.c`. The matrix runner's own exit 1 is the
candidate-failure INVENTORY, not an evidence failure;
`scripts/ci_gcc17_torture.sh` treats 1 as advisory and only 2 as invalid.
S09 (after the §9.7 peephole optimisation) has **byte-identical counts and
an EMPTY S08 -> S09 per-case diff on both legs**: the assembly-equivalence
cache changed no corpus verdict across a further 4032 case-runs.

### 9.6 Local CI contract: three REAL contract defects found and fixed

Running `scripts/ci_local.sh --fast` on the S02 revision (which had never been
done) exposed three defects that the earlier targeted checks had missed. All
three are fixed here with measured evidence, not by relaxing a gate:

| gate | defect | fix |
|---|---|---|
| `driver-option-contract-core` | the gate still demanded `-fopenmp` be REFUSED; S02 implemented it (`_OPENMP 202111`) | removed the flag from the refusal list and added a POSITIVE contract: `-fopenmp -dM -E` must define `#define _OPENMP 202111`, `-fno-openmp` must not. The 31-case `check_vectorizer_openmp_policy.sh` remains the full policy gate. |
| `attribute-class-diagnostics` | the `mode` fixture used `int f(void) __attribute__((mode(DI)));` and expected SILENCE, but gcc 16.2 refuses that spelling with `mode 'DI' applied to inappropriate type` | moved the fixture to the position gcc actually accepts silently (`int x __attribute__((mode(DI)));`) and added a class-1b assertion pinning the refusal AND wording on the function declarator. |
| `audit-census-ratchet` | `parse_external_decl` grew from 298 to 324 lines, so `fns_over_300` went 232 -> 233 | extracted the C89 implicit-int fallback into `Parser::implicit_int_type_specifier`. Census is back at the 232 baseline; NO re-baseline was used. |

**Full local verification after those fixes** (fastbuild, 4 GiB swap,
gcc-16.2 + clang-23.1.4 + rust-1.99.1 all on `PATH`):

* `cargo fmt --all --check` PASS; `check_clippy_policy.py` PASS (76 lints, 0
  unknown, 0 warnings); `cargo test --profile fastbuild --all-targets` PASS
  (**7941 passed / 0 failed / 10 ignored**, zero compiler warnings/errors);
* `audit_census.py --check` PASS (all nine metrics held);
* `check_ci_gate_parity.py` PASS (538 commands / 27 contracts),
  `check_ci_workflow_shell.py` PASS (14 workflows / 2 rules),
  `check_ci_lane_plan.py` PASS (frontend-a 360.43 s / 59 steps),
  `check_ci_shard_partition.py` PASS (85 / 116 disjoint gates),
  `check_pipefail_sigpipe.py` PASS (344 scripts),
  `check_errexit_negation.py` PASS (540 scripts),
  `check_option_effect_ledger.py` PASS (10 / 10, GCC 16.2.0),
  `check_doc_links.py` PASS, `check_pr_claim_fidelity.py` PASS;
* the four new gates: asm constraints **21/21**, GNU declaration corners
  **22/22**, parser depth **4/4**, vectorizer/OpenMP policy **31/31**;
* inline-asm `validate()` unit tests **42 / 42**, and a 64-spelling
  accept/reject differential against gcc 16.2 with **0 divergences**.

**Host-prerequisite classification (data-driven, not handwaving).** The first
`--fast` run used only gcc-16.2 on `PATH` and reported 39 red gates. Re-running
all 39 individually with the Clang 23.1.4 bin directory on `PATH` turned **22
of them green** (`oracle-toolchain`, all seven `ext-vector-*`,
`microsoft-scalar-abi`, `warning-precedence`, `warning-frame-size`,
`attribute-diagnostics`, `typed-constant-truth`, `count-observations`,
`narrow-int-codegen`, `arithmetic-test-sign-cmov`, `fma-contract-gating`,
`sse34-clang-encoding`, `incoming-byval-homes-clang`, …). The remaining 17 are
**only** missing external tools that hosted CI provisions and this sandbox's
network policy blocks, each identified by the failing command:

| missing tool | gates | provenance attempted |
|---|---|---|
| `qemu-i386` / `qemu-aarch64` / `qemu-riscv64` | `kernel-codegen-contracts`, `enum-forward-callback-abi`, `pr824-boundaries`, `linker-suite` | `apt-get` has no package lists and no reachable mirror; not in the `harness-cache-4` release assets |
| `mold` | `gnu-retain-sections` | same |
| GNU as/objdump 2.47 | `asm-diff-oracle-gas-2.47`, `i686-gnu-vector-abi`, `a64-gas-provision`, `encdiff-*`, `asmdiff-*`, `x86-asm-diff`, `i686-asm-diff` | `ensure_gas_247.sh` tries `mirrors.kernel.org` / `mirror.csclub.uwaterloo.ca`; both TLS-reset in this sandbox. A GitHub source tarball would NOT hash to the pinned immutable GNU release bytes, so it was deliberately not substituted |
| ARM A64 ISA XML 2026-09 | `arm-a64-xml-provision`, `aarch64-arm-doccheck` | `developer.arm.com` TLS-reset |
| Linux `relocs` tool | `kernel-relocs-tool`, `linker-suite` | `git.kernel.org` TLS-reset |
| rustc 1.97.0 via rustup | `msrv-check` | not installed; hosted MSRV job is green |

None of these is a compiler defect, and NO gate was weakened to hide them:
hosted CI runs exactly the same `--fast` gates with these tools provisioned.
The local evidence is therefore "every gate whose prerequisites exist PASSES",
and hosted CI is the authority for the rest. `linker-suite` itself reports
**335 pass / 0 fail / 6 SKIP** even locally; its exit 1 comes from the
`--require` host-tool check, not a linker regression.

**ICX honesty:** `kernel-codegen-contracts` and `enum-forward-callback-abi`
print `UNMEASURED: ICX oracle (not installed)`. The hosted
"Authenticated Intel ICX 2026.2 integration" job is green on this branch and
is the third oracle; no local ICX 2026.2 binary exists in this sandbox, so no
ICX-witnessed numerical claim is made here.

### 9.7 S08/S09: red-teaming the K&R fix, and an O(F^2) host-CPU hotspot

**The most important finding of this round is a red-team of our OWN fix.**
The first repair of `991213-2.c` made every one of 23 declarator shapes
*accept* exactly as gcc 16.2 and clang 23.1.4 do, and the whole 2016-case
matrix went green for that file. It was still WRONG. Acceptance parity is a
weak oracle: gcc types `int (* const (p)(int))` as `int *const (*)(int)` --
the stars and their qualifiers bind to the FUNCTION'S RETURN type, and C11
6.7.6.3p8 adjusts the function-typed parameter to a pointer. The first fix
produced `int (*const)(int)`. Both spellings compile a declaration, so only a
*discriminating* program exposes the difference:

| probe | gcc 16.2 | clang 23.1.4 | first fix | final |
|---|---|---|---|---|
| `int g(int (* const (p)(int))) { return *p(1); }` | OK | OK | ERR | OK |
| `int g(int (** (p)(int))) { return **p(1); }` | OK | OK | ERR | OK |
| `int g(int (* const (p)(int))) { p = 0; return 0; }` | OK | OK | ERR | OK |

The oracle type was pinned, not guessed: `__builtin_types_compatible_p(
typeof(p), int *(*)(int))` and `int *const (*)(int)` both hold under gcc 16.2,
while `int (*)(int)` does not; `sizeof(p)` is a pointer; clang 23.1.4 prints
the parameter type as `int (*const (*)(int))`. ICX 2026.2 was not installed in
this sandbox (Intel PPA unreachable, release cache contains only clang/rust),
so **no ICX-witnessed numeric claim is made locally**; hosted CI's
"Authenticated Intel ICX 2026.2 integration" job is the third oracle.

**Seven code fixes, each with a measured differential:**

1. `parse_paren_param_declarator` adopted the nested `(name)` group's own
   parameter list instead of consuming the enclosing `)` optimistically and
   rewinding (the original `expected ')' before '('`).
2. Its stars now bind to the RETURN type: one leading pointer on the base plus
   a decaying function core, with `fptr_inner_ptr_depth = 0` and
   `parsing_pointer_const` cleared. The generic tail adds
   `inner_ptr_depth - 1` more, so `(** (p)(int))` gets full depth.
3. Two byte-identical array-dimension loops were folded into
   `parse_paren_array_dims`, and the six-way `plain_stars` predicate into the
   pure, unit-testable `nested_group_is_plain_stars` (8 unit tests, one per
   disjunct). This keeps `parse_paren_param_declarator` UNDER the
   AUD-RATCHET-1 300-line ceiling; the census stays at its 232 baseline,
   no re-baseline.
4. `T *const p(int)` (function-typed parameter): the leading star's `const`
   qualifies the RETURN pointer, not the parameter object. Leaving the flag
   set produced a bogus `assignment of read-only parameter`.
5. `int (*const p)(int)` (function POINTER): the group's single star IS the
   parameter object, so `p = 0` must be refused. Sema's `top_const` now
   consults `pointer_const` for `fptr_params` + `paren_star_quals` too.
   Both directions are pinned: 12 const-placement shapes x 3 compilers x 2
   ABIs, 0 divergences.
6. C11 6.7.2.1p2: a struct member cannot have function type. `struct S {
   int m(int); };` and the K&R `int (*const (m)(int));` are now refused,
   while `int (*m)(int)` (ends in `FunctionPointer`, not `Function`) stays
   legal.
7. C11 6.7.6.3p7/p1: `static`/qualifiers in a non-parameter array
   derivation and `(*p)(int)[4]` (function returning an array) are now
   refused with the same verdict as both oracles. The legal
   `(*const (p)(int))[4]` and `int a[static 4]` are untouched.

**Gate C in `check_gnu_declaration_corners.sh`** (already wired into both CI
mirrors, so no orchestration churn) has three layers: 44 acceptance shapes +
12 const shapes + 7 type-sensitive bodies + 11 C99/C11 spellings = 74 shapes
x 2 ABIs x 2 available oracles = **296 verdict comparisons**; and 3
end-to-end fixtures compiled at `-O0/-O1/-O2/-O3` by lccc, linked by gcc
16.2 and EXECUTED on the host CPU (**18 runtime checks**, zero diagnostics
allowed). The `int (* const (p)(int x)) { ... return &cell; }` fixture is a
real K&R function DEFINITION whose `*p(0)` only type-checks if `p` returns
`int *`; the wrong type cannot compile it. A missing optional oracle
(clang, icx) is PRINTED as unavailable, never silently counted as agreement.

**Host-CPU deep dive (measured, not guessed).** Callgrind on 250 K&R
declarations: 4,737,746,618 instructions, 18.64% inclusive in
`label_mention_counts`, which issued **4,394,006** `find(".LBB")` calls.
Root cause: two peephole passes recomputed a WHOLE-TU label scan for every
candidate attempt, making them O(F^2) in the number of functions. Both
`hoist_one_arm_head` and `conditional_tail_calls_in` are side-effect free on
every non-rewrite path and mutate only when returning/applying a rewrite, so
caching `(count, duplicate)` across failed attempts and invalidating on the
first success is provably value-identical.

| N K&R functions | before (O2) | after (O2) | gcc 16.2 | clang 23.1.4 |
|---|---|---|---|---|
| 250 | 0.293 s | **0.233 s** | 0.355 s | 0.079 s |
| 500 | 0.772 s | **0.520 s** | 0.682 s | 0.136 s |
| 1000 | 2.336 s | **1.052 s** | 1.327 s | 0.259 s |
| 2000 | 6.619 s | **2.172 s** | 2.650 s | 0.533 s |

Per-function cost went from superlinear (1.17 -> 3.31 ms) to essentially
FLAT (0.93 -> 1.09 ms); **-67.2% at N=2000, and lccc now compiles this
workload faster than gcc 16.2 (0.82x)**. Clang 23.1.4 remains ~4x faster on
frontend throughput; we do NOT claim to beat it. Generated-code host runtime
on a 40M-iteration integer kernel (median of 5, same object linked by gcc
16.2): lccc 0.0300 s, gcc 16.2 0.0282 s, clang 23.1.4 0.0169 s -- lccc is
**NOT** faster there either, and beating the other two on code quality is a
separate optimisation program, not a claim made by this patch.

**Equivalence proof for the cache.** The pre-optimisation and post-
optimisation binaries were built from the SAME tree with only
`jump_thread.rs` swapped (`98ab3736…` vs `f4acdb67…`), then both emitted
`-std=gnu11 -O2 -S` for 159 sources (9 branch-heavy hand-written fixtures
plus 150 randomly sampled torture files): **138 byte-identical, 0 different,
21 skipped (both binaries reject them), 0 timeouts**. Three deterministic
unit tests pin the structural property: with 64 straight-line functions each
pass performs <= 1 whole-TU scan (64 before), and the rewrite-heavy golden
body still reaches a fixpoint. A structural call-count assertion was chosen
over a wall-clock threshold because CI timing noise would make the latter
flaky -- "no cycles left behind" needs a proof, not a race.

**Verification after all fixes:** `cargo fmt --all --check` PASS;
`cargo clippy --all-targets -- -D warnings` PASS with 0 warnings;
`check_clippy_policy.py` PASS (76 lints); full unit suite **7952 passed /
0 failed / 10 ignored**; `audit_census.py --check` PASS (all nine metrics
held, no re-baseline); nine policy checks PASS; four gates PASS
(asm 21/0, GNU corners 24/0, depth 4/0, vectorizer/OpenMP 31/0).
`origin/main` is still `49489a5d…`, i.e. this branch is already based on
the latest upstream main (0 behind). Hosted CI on the preceding revision is
green on 24 checks (including the ICX 2026.2 integration job) with the
kernel job pending; the new push re-runs the full matrix.


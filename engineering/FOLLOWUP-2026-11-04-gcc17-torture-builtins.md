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

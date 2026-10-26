# S11 red-team audit — 2026-10-25

Scope: every change in `/home/user/ms178-1.patch` (base `abe67eaa`, head as recorded in the session snapshot ledger kept
outside the repo), the earlier S09/S10 work it is
rebased onto, and the claims the earlier team made about both.

The rule applied throughout: **a claim counts only if it is reproducible from
this tree with the tools that are actually installed.** Where the pinned oracle
(GCC 16.2, Clang 23.1.3, as 2.47, mold 2.42.1) is absent, the row says so
instead of quoting a number from a log I cannot re-derive.

---

## 1. Environment reality (read this before any performance claim)

| tool | pinned | present here | consequence |
|---|---|---|---|
| GCC | 16.2 | **14.2.0 (Debian 13)** | every `check_*.py` gate pinned to gcc-16 refuses to run; no post-fix performance claim is possible |
| Clang | 23.1.3 | 19.1.7 | ditto for the Clang-comparison gates |
| binutils `as` | 2.47 | 2.44 | `i686-gnu-vector-abi` cannot be validated |
| mold | 2.42.1 | 2.37.1 | link-time comparison not repeatable |
| ICX | 2026.2 | absent | Intel rows unverified |
| valgrind/callgrind | required | 3.24.0 (**installed this session**) | available for the next turn |
| gcc-multilib | required | **installed this session** | i686 gates runnable |
| aarch64 cross | required | 14.2.0 (**installed this session**) | cross-builds runnable |

Measured, this session: `bash scripts/ci_local.sh --fast` →
**646 PASS / 92 FAIL**, and every one of the 92 names an absent oracle
(`gcc-16`, `GCC 16`, `bits/libc-header-start.h` before the multilib install,
`-lgcc`, Clang 23). After installing gcc-multilib, the multilib-dependent
failures lose their cause; the gcc-16-pinned ones remain until GCC 16.2 is
installed. **I make no "green" claim for a gate that cannot run.**

What DID run green on this tree, so the record is complete:

| check | result |
|---|---|
| `cargo test --lib -j2` | 6377 passed, 0 failed, 10 ignored |
| `cargo clippy --all-targets` | 0 warnings, 0 errors |
| `cargo fmt --check` | clean |
| `python3 tests/regression/run_regression.py --gcc gcc` | **1151 passed, 0 failed**, 3 skipped-compare (gcc cannot parse those) |
| `scripts/audit_census.py --check` | no metric grew |
| `scratch/check_f1_compound_literal.py <lccc> gcc` | 5/5 rows agree |
| `cases.tsv` all 20 rows, compiled AND run by both compilers | all agree |

The pre-wipe claim "fast CI 441/0/6 GREEN" is not reproducible here and should
be treated as historical, not as evidence.

---

## 2. What the earlier team got right (and I kept)

1. **The `value_home_is_expendable` direction was right, the predicate was
   wrong.** Treating a hole after the clobber point as "dead" is exactly the
   `vc_handle_mmio` miscompile (`%r14` homed, segments
   `[(7,9) (11,21) (23,23) (119,127)]`). I agree with the fix, and I kept it.
   What I changed: the recovery test was a **one-hop** match on the *kind* of
   the defining instruction. `v = copy t; t = copy s; s in a slot` is
   recoverable and was refused; `v = copy t` with `t`'s home clobbered is NOT
   recoverable and was accepted. Both directions were wrong, so the rung is
   now a transitive walk (`value_has_static_recovery`, depth 4, cycle-safe,
   stops at a clobbered link).
2. **The switch `direct32` analysis** (`subl` is the stale-mark source, and
   only 32-bit index classes may use the in-place form) is correct and matches
   the emitted assembly. Kept.
3. **The dense `LabelMap`** decision is right: labels grow by the number of
   cloned blocks, not by the label value. I disagree only with the capacity
   story, see §3.4.
4. **The compound-literal-as-element** work (`compound_literal_items`,
   `[i] = (T){ ... }`) is correct for the *element* case and I kept it.

---

## 3. What was wrong, and why (all five are fixed in the patch)

### 3.1 An array-typed compound literal never initialized an array
Five independent defects, each silent (no diagnostic, wrong bytes):

* **Member level.** `struct { unsigned long pad; int m[2][3]; } x = { .m =
  (int[2][3]){ {1,2,3},{4,5,6} } };` left `m` **all zero**: the literal was
  read as one scalar expression of the member. The array-of-aggregate flavour
  did not even terminate (the literal re-entered the same field loop for the
  same item) — a compiler hang, found only because I bisected the shape.
* **`[i].field = (T){...}`** filled at the *element* offset with the
  *element's* layout. With `struct { unsigned long pad; nm_t n; }` the
  literal's first member landed on `pad` and `n` stayed zero. This is the
  worst class in the patch: a silent wrong value where the previous code
  produced a *diagnostic*.
* **Canonicalisation.** Brace elision (6.7.9p20) swallowed the literal and
  stored it as the array's **first scalar**.
* **Type cache.** `get_expr_ctype` keys its cache by AST **heap address**, so
  two compound literals in one TU can share a key: `(char[4]){"ab"}` inherited
  the type of a preceding `(int[2][3]){...}` and its member stayed zero. This
  is an ABA hazard the file already documents for `Cast`/`Deref`; the literal
  is the third case and now bypasses the cache like they do.
* **Object level.** `nm_t g[2] = (nm_t[2]){...}` (file scope, and block-scope
  `static`) was rejected as non-constant, where GCC accepts it.

Evidence (all rows are compiled AND run by both compilers):

| shape | gcc 14.2 | lccc before | lccc after |
|---|---|---|---|
| `.{member} = (int[2][3]){...}` | 61 | 0 | 61 |
| `[1].a = (nm_t[2]){...}` | 134 | 100 | 134 |
| `[1].n = (nm_t){.b={[1]=6}}` (`pad` at 0) | 106 | 6 | 106 |
| `(char[4]){"ab"}` member after an `int[2][3]` literal | 71 | 70 | 71 |
| `nm_t g[2] = (nm_t[2]){...}` | 709 | rejected | 709 |
| block-scope `static` array literal | 70913 | rejected | 70913 |
| three literals in one TU | 136071 | 134075 | 136071 |

`tests/regression/compound_literal_element/cases.tsv` grew 8 → 20 rows, and
**every** `# probe:` value in it is now a measured value (the file previously
carried two that neither compiler produced — I verified all 20 rows against
both compilers, and that is what caught it).

### 3.2 `sema` accepted what it must reject, and rejected what it must accept
* Block-scope `auto` array initialized by an array expression
  (`int a[3] = (int[3]){1,2,3};`) was **accepted and left uninitialized** —
  the program read garbage. GCC rejects it ("array initialized from
  non-constant array expression"), and so does lccc now; `int b[3] = a;` gets
  GCC's other wording, "invalid initializer".
* The `static`-initializer check treated **every** array literal as a decayed
  address, so the legitimate `static int a[3] = (int[3]){1,2,3};` was
  rejected. `non_constant_leaves_for` now takes the object's type.
* **Known, documented deviation (not fixed):** a literal that initializes a
  *member* array of a static object (`static w v = { .a = (nm_t[2]){...} }`) is
  still rejected. Fixing it needs the member's type inside the constant walk;
  the check declines rather than guesses. This is the only remaining
  accept/reject divergence I know of in this area.

### 3.3 Function-pointer parameters lost their type
`typedef int (*fp_t)(int (*)(unsigned int), int[3]);` resolved its nested
function-pointer parameter as `int *`, so
`__builtin_types_compatible_p(fp_t, int (*)(int (*)(unsigned int), int *))`
answered **0 for two identical types** — at file scope through
`SemaConstEval`, and at block scope through the lowering's typedef path
(`build_function_pointer_ctype_from_typedef`). Both now resolve parameters
through `param_decl_to_ctype_with`. Verified: `1 1 1 0 1` before and after on
the control matrix, `a=1` where it was `0`.

**Agreement/disagreement:** the earlier team listed this as "fixed" before it
was. Their fix touched only the two `TypeSpecifier` arms; the typedef path that
a kernel callback table actually exercises was still wrong, and the file-scope
half only looked fixed because the sample they used had no nested parameter.

### 3.4 Inline-asm register discipline
* i686 `resolve_memory_operand` could return with `mem_addr` **empty**, so the
  template emitted `lock btsl $0,` — the exact failure mode recorded for
  zsmalloc on x86-64, still live on i686. Now materialised through
  `operand_to_eax`, and **declined** when `%eax` is reserved (the caller's
  fallback is safer than overwriting a live operand).
* The x86-64 pool-exhaustion path returned a **hardcoded `%rcx`**, i.e. a
  register that an explicit constraint may be holding. Now the first
  non-excluded scratch register.
* `busy_regs` reserved only "specific registers + operand registers", but
  phase 1f materialises memory addresses **before** phase 2 loads inputs, so a
  register-homed input was invisible. `reserved_home_regs` closes it.
* `LABEL_MAP_RESERVED` 2 is wrong: a merge block plus the on-demand
  `LabelMap::get` mints (asm goto, terminator-only references) is up to 4. And
  the base check compared against `u32::MAX` *without* the clone's own block
  count, so the "capacity pre-check" it claimed was not a capacity check at
  all. Both fixed; `assign_fresh` asserts rather than wrapping.

### 3.5 GAS scanner
`strip_comment` and `split_on_semicolons` handled `"..."` but not **character
constants**. `movb $'#, %al` lost everything from `#`, and `$';` split a
statement in half. Both scanners are now character-literal aware (unit-tested).

---

## 4. Self-criticism of this pass

1. **I re-derived work that was already lost.** The harness wiped `.git` and
   the tree between turns; the initializer fixes had to be re-applied from
   scratch, and I initially reported them as done when only the *pre-wipe*
   tree had them. The lesson is mechanical: **grep the tree for the marker
   symbol before claiming a fix is present**, which is what I now do
   (`git status --short` + `grep -c <symbol>`).
2. **Two `cases.tsv` probe values were wrong in the file I was extending.** I
   inherited them and only a mechanical re-measure found them. Every new row
   is now measured, not written.
3. **The `sema` automatic-array rule is a behaviour change for code that
   compiled before.** It matches GCC, but it is the kind of change that must be
   declared: it is in the commit message and here.
4. **The F1 harness versions.** The repository gate pins GCC 16.2 and refuses
   to run here, so the A/B matrix above was produced by
   `/home/user/scratch/check_f1_compound_literal.py` against GCC **14.2**. For
   these C11 constant-expression shapes I expect 16.2 to agree (I verified the
   same matrix by hand against 14.2 and by construction against the standard),
   but **this is not an oracle-grade claim** and must be re-run with 16.2.
5. **No performance claim in this pass.** Correctness-only changes cannot
   improve codegen; the two codegen changes (recovery ladder, scratch pools)
   are fail-closed by construction and cost at most one register-to-register
   move on a path that previously could not compile.
6. **The kernel gate did not run this session** (no kernel source on disk,
   3.9 GB free, 2 GB RAM). The claiming rule applies: no kernel claim without
   the build.

---

## 5. What I recommend next, in order

1. Install GCC 16.2 / Clang 23.1.3 / as 2.47 / mold 2.42.1, then re-run:
   `check_compound_literal_element.py`, `ci_local.sh --fast`, and the Callgrind
   matrix. Until then no oracle-grade statement is possible.
2. Kernel 6.18.58: full build + 32 KiB boot gate with the new `sema` rule
   active — the automatic-array diagnostic is the change most likely to meet
   kernel code that previously compiled.
3. Member-array static initializer (`static w v = { .a = (T[2]){...} }`) —
   the one documented deviation left.
4. Then performance: `worst-ten` still lists `byte_load_reassembly` (2.46×
   GCC) and `ring_fifo` (2.38×) from the pre-wipe Callgrind run; both are
   real, reproducible failures to investigate, and neither is touched by this
   patch.

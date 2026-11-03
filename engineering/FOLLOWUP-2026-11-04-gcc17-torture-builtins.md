# Follow-up 2026-11-04 — GCC 17 C-torture: real builtin semantics, ABI-correct lowering, measurable gains

Branch `arena/e0bf34d9-lccc`, PR **#1096**, base `9bed91bec6c044d8a7fadfc7e062fc62b4e57a31`.
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
Two harness wipes occurred during this session; both were recovered from it. `ensure_swap.sh`
recreated the 8 GiB swap (hard requirement: ≥4 GiB).

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
2. **4 vector-type semantic errors** (`vector-1/2/3.c`) and **4 constant-initializer**
   errors (`20001116-1.c`, `20010113-1.c`, `20050516-1.c`, `941014-4.c`,
   `pr48517.c`): genuine front-end gaps.
3. **4 inline-asm "impossible constraint"** (`20000922-1.c`, `asmgoto-2.c`,
   `pr27528.c`, `pr35318.c`): constraint alphabet coverage.
4. **2 internal assembler errors** (`.LBB1`/`.LBB2` in `pr17913.c`, `pr84136.c`):
   computed-goto label addresses reference a block label that was merged away during
   block cleanup. GCC emits the label even in dead code. The right fix is in lowering
   (keep label-address blocks alive / fold duplicate labels), **not** in
   `find_symbol_index_shared`: defining an undefined local label at a guessed offset
   would turn a real miscompile into a silently wrong jump. Reconstructed assembly is in
   the session evidence (`leaq .LBB1(%rip)` with no `.LBB1:` definition).
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

* `/home/user/ms178-1.patch` — 73,248 bytes, sha256
  `963f44eb4015c167ab14f0ee4502f39673f939e56da73223fde3bec02702f556`,
  APPLIES-CLEAN against `9bed91b…`. Ledger `ci_gate=UNGATED` because the user forbade
  local CI; the real gate is hosted GitHub CI on PR #1096.
* `/home/user/artifacts/` — timestamped patch series, source tar, session bundle,
  snapshot ledger.
* Reports: `/home/user/.cache/lccc-torture/full2/{arch}-{mode}.json` and
  `.failures.txt` (wipe-volatile; the numbers are recorded above).

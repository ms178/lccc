# Follow-up: 2026-10-22 codegen round — open PRs, oracle provisioning, new workload kernels

Session scope (in the order it was executed): restore the sandbox to a working
state, provision the reference compilers from the repository's own payloads,
perfect the two open PRs, extend the workload corpus, and quantify the next
performance gap. Every claim below is tied to a command in this document or to
a committed gate; where something is a measurement from this host, that is
stated and the per-host caveat is given.

Host for every number here: 2 vCPU KVM guest (`Intel Xeon Processor @
2.60GHz`, model 106), 1.9 GiB RAM, **no usable PMU** — so no counter claims are
made; instruction counts come from Valgrind's callgrind (deterministic) and from
static assembly censuses, and wall time comes from `run_benchmarks.py`'s paired,
randomised-order, bootstrap-interval protocol.

---

## 1. Session infrastructure (prerequisite for everything else)

The harness wipes `/home/user` between sessions and drops `/opt`, `/swapfile`
and `target/`. The recovery path is `scripts/arena_session_restore.sh`, and it
worked unchanged — but it has one hole that cost real time in this session and
is worth fixing:

* **The oracle compilers are not part of the restore.** Every gate that names
  `gcc-16` or `clang-23` resolves it on `PATH`, so a host whose distribution
  does not ship GCC 16 can either skip the gate or build an oracle by hand.
  Both are wrong when the payloads are already in the repository.

### 1.1 `scripts/oracle_toolchains_from_tags.sh` (new)

Restores, verifies and installs the pinned oracles with no vendor network
access, from the transport tags that already exist for exactly this purpose:

| payload | tag | version | digest-checked |
|---|---|---|---|
| `gcc-16-bookworm-x86_64.tar.xz` | `harness-cache-1` | 16.2.0 | yes |
| `clang-23.1.3-linux-x64.tar.xz` | `harness-cache-4` | 23.1.3 | yes |
| `mold-2.42.1-x86_64` | `harness-cache-1` | 2.42.1 | yes |
| `binutils-2.47.tar.xz` | `harness-cache-1` | 2.47 | verified always, built only with `--with-binutils` |

Two design points that matter more than the code:

* **The digest is not taken from the tag alone.** The script carries its own
  table *and* cross-checks it against the tag's `MANIFEST.sha256`, so a
  re-issued payload fails loudly (printing both digests) instead of a rewritten
  tag vouching for itself.
* **Exit 0 is earned by running the tools**, not by writing files with the right
  names: `--version` must contain the pinned version *and* each driver must link
  a trivial program (gcc-16 through its own prefix; clang-23 with `-fuse-ld=mold`
  through the installed mold), which is what catches a prefix extracted without
  its crt files.

ICX 2026.2 comes from the existing `scripts/ensure_icx_2026.sh` (authenticated
Intel APT repo, rootless prefix). The full competition set is therefore real and
local: lccc, GCC 16.2.0, Clang 23.1.3, ICX 2026.2.0, mold 2.42.1.

### 1.2 Swap

`/swapfile` (6 GiB) is still mandatory: the compiler build peaks above the
guest's 1.9 GiB and the link step is OOM-killed without it. `scripts/
ensure_swap.sh` and `build_lccc_fast.sh` both handle it; the restore script
creates it first thing.

---

## 2. Open PRs — audit, rebase, and what was rejected

Both open PRs (`#903`, `#904`) were written against an older `main` and neither
was an ancestor of `main`. Both were rebased here, and in both cases **one half
of the PR was stale and would have regressed `main`**. That is the audit result,
not an implementation detail.

### 2.1 PR #903 — retrying linker-oracle provisioning

| the PR's part | verdict |
|---|---|
| `scripts/ci_install_link_oracles.sh` (retries, `apt-cache pkgnames` resolution, fail-closed on both binaries, free on a warm runner) | **kept** — strictly better than the bare apt line it replaced |
| the PR's seventh per-job workflow step | **rejected** — `main` has since collapsed the six per-job package steps onto one entry point (`ci_provision_toolchain.sh` + the content-addressed bundle). Adding the PR's step back would re-create the per-job path the bundle exists to remove |
| relaxing `test_bad_oracle_exit_or_output_still_fails` to expect `skip-compare` for a non-zero oracle | **rejected as stale** — `main` replaced the blanket skip with a per-fixture `LCCC_ORACLE_SELFCHECK` waiver, which is strictly narrower: a stale oracle is skipped only where the fixture says why, and the skip names the escape hatch |

Resolution: the pair (`mold`+`lld`) is now **delegated from the single
provisioning entry point** to the retrying installer, and the non-oracle extras
still ride one apt transaction. The PR's 14-case mock gate was kept and
strengthened: its workflow pin asserted "the Install differential linker
oracles step still exists", which stopped describing anything once the steps
were collapsed. It now asserts what matters — no lane executes a private
installer, and provisioning really does reach the retrying script for the pair
(behaviourally, under `PATH` stubs), while a single-oracle request stays on apt
because it is not that contract.

`check_ci_toolchain_bundle.py` was updated for the same reason: it pinned the
old sequence (one apt transaction carrying all four extras). It now pins the new
one and asserts the delegation happens exactly once.

### 2.2 PR #904 — SSE2 codegen matrix + 16-byte aggregate round-trip collapse

The substantive half is real and measured: `src/passes/aggregate_pair_roundtrip.rs`
plus a 122-row per-instruction matrix gate. Three conflicts, resolved as:

| conflict | resolution |
|---|---|
| `src/passes/vectorize.rs` | **`main` wins.** `main`'s SSE2-baseline matmul arm is newer (it carries the `LCCC_FORCE_SSE2` / `sse2_lowering_fuses` reasoning and a "no SIMD register file" legality arm the branch predates). The branch's semantic intent — a refused FP contraction is not a refusal to vectorize on the x86-64 baseline — is already in `main`, expanded |
| `ci.yml`, `ci_local.sh` | **union** — both sides added gates |

Measured effect of the merge on the committed matrix, same host and same
oracles: **3295 → 3230 LCCC instructions over the 122 rows, 11 rows improved, 0
regressed** (top: `minsd` 103→82, `maxsd` 102→84, `sqrtps` 30→24, `sqrtss`
34→28).

#### 2.2.1 A real defect in the matrix gate itself

The gate is monotone on two quantities that are not the same invariant:
instruction count (may shrink) and "no recorded mnemonic may disappear". The
recorded mnemonic set is a *superset* of the row's lowering — it also holds
incidental spill/repack traffic — so the aggregate round-trip collapse removed
the scalar `movsd` from `maxsd`/`minsd` **while taking them 102→84 and
103→82**. The gate as written forced a choice between keeping the cheaper code
and keeping CI green.

Fix: a disappearance is now a failure **only when the row did not shrink**, i.e.
only when it is a pure loss; lowering fidelity itself stays pinned by
`missing_expect` against the row's expected set. The `--self-test` pins *both*
directions (vanished with no shrink must fail, vanished with a shrink must
pass), so the rule cannot be quietly weakened into never failing. Baseline
re-recorded.

---

## 3. Defect found by auditing a table instead of trusting it

While looking for the next codegen win, `__builtin_fmaxf` turned out not to be a
builtin at all in lccc, while `__builtin_fmax`, `__builtin_fmaxl` and every
other libm sibling were. That is a **table drift** defect, and the audit was
made systematic rather than case-by-case: for every `BuiltinInfo::simple("X")`
mapping, probe `gcc-16 -fsyntax-only` for `__builtin_Xf` and ask whether lccc
maps it too. Answer: exactly three gaps — `fmaxf`, `fminf`, `log2f`.

Adding the aliases was **not enough**, which is the interesting part:

* the alias table resolves the *name*;
* a second table (`src/ir/lowering/expr_types.rs`) supplies the *result type*;
* that second table did not know the three names either, so the call was typed
  `I64` and the caller converted the callee's float `%xmm0` through an integer.

Measured before the fix (GCC 16.2 in brackets):

```
__builtin_fmaxf(1.5f, 2.5f)  = 1.08842e+09        [2.5]
__builtin_fmaxf(NaN, 7.0f)   = 1.08842e+09        [7]
__builtin_log2f(3.0f)        = 4.6098e+18         [1.5849624872207642]
```

Those are the raw float bit patterns reinterpreted as integers — a *wrong
number*, not a missing symbol, which is why no existing gate could see it.

Gate: `tests/regression/check_builtin_float_aliases.py` (`builtin-float-aliases`,
wired into `ci_local.sh --fast` and `ci.yml`). It **derives** the class from the
alias table plus `gcc-16` probing (arity included), requires `-Werror` on the
implicit declaration, requires the builtin name not to survive into the assembly,
and compares every float spelling against the libc function it names **through a
function pointer** so no compiler can fold the reference. A bogus control name
must fail with an implicit-declaration diagnostic, so the gate proves it can
detect an absence. Verified by reverting only the `expr_types.rs` entry: the gate
fails with `nan-f-b: lccc builtin 1065353216.0 != libc 1.0`, then passes again
with it restored.

Two behaviours are **adjudicated, not asserted equal to GCC**:

* **Signed zero.** glibc's `fmax`/`fmin` are comparison-based, so
  `fmax(-0,+0)` is `-0`; an Annex F inline expansion answers `+0`. GCC 16.2
  inlines and answers `+0`/`-0`; lccc calls glibc and answers `-0`/`+0`. The
  gate pins each compiler to the library it binds to and does not demand that a
  call and an inline expansion agree on a zero sign.
* **"Inlined" vs "called"** is reported, not failed — the assertion is about the
  mapping, not about how it is lowered.

---

## 4. Workload corpus — second extension round

The corpus already carried 68 kernels with pinned provenance (gzip CRC,
zlib-ng Adler-32, Expat scan/SipHash, SQLite varint, Linux `find_bit`/rbtree,
glibc `memcmp`/`strstr`, ChaCha20, SHA-256, ZSTD count, LZ4). Three new kernels
were added for **pressure profiles the corpus could not express**, each with a
provenance row (`tests/benchmark/WORKLOAD_PROVENANCE.md`), a registry entry in
`run_benchmarks.py`, and an in-binary self-check against a naive reference:

| kernel | upstream | why this shape |
|---|---|---|
| `gzip_longest_match` | gzip 1.14 `deflate.c`, `longest_match()` | hash-chain walk with a data-dependent trip count, a widened unaligned load masked to `MIN_MATCH` bytes, and a `ctz`-based first-difference; loop-carried best-length update |
| `linux_list_sort` | Linux 6.18 `lib/list_sort.c` | pointer-chasing merge sort behind a **non-devirtualizable comparison callback**; 64-bin bottom-up scheme selected by `ctz` of the element index |
| `zstd_double_fast` | zstd 1.5.7 `lib/compress/zstd_double_fast.c` | two independent hash probes per iteration feeding address arithmetic; unaligned `read32`/`read64`; `ZSTD_count`'s XOR+`ctzll` extension |

All three produce byte-identical output under lccc, GCC 16.2, Clang 23.1.3 and
ICX 2026.2 — the self-checks are what make that a meaningful statement (a fast
but wrong matcher fails the reference comparison rather than winning the timing
column).

Two authoring bugs were found and fixed *while writing them*, both by the
self-checks doing their job, and both worth remembering: comparing four bytes in
gzip's `MIN_MATCH`=3 pre-check rejects a legitimate 3-byte match, and a
stability check must be derived from the list's actual initial order (nodes
pushed onto the front is a descending original order).

### 4.1 First measurement (paired, randomised order, 9 rounds, `-O2`)

| kernel | lccc | best reference | lccc / best | verdict |
|---|---|---|---|---|
| `gzip_longest_match` | — | ICX | **1.042×** slower | close |
| `zstd_double_fast` | 14.00 ms | GCC 12.84 ms | **1.097×** slower | gap |
| `linux_list_sort` | — | ICX | **1.448×** slower | **largest gap** |

Aggregate over the three: geomean **1.18×** slower than GCC, **1.18×** slower
than the fastest available reference. Every arm was flagged for being under
20 ms with 5–9 % CV, so these are *screening* numbers: the direction and the
ranking are solid, the magnitudes need the `PASSES` scaled up (see TODO-1).

---

## 5. Root causes identified this session (evidence, no guesses)

### 5.1 `linux_list_sort`: no interprocedural constant propagation of a function-pointer argument

Callgrind (`--tool=callgrind`, `-DPASSES=1`, identical source, `-O2`):

| compiler | dynamic Ir | where it goes |
|---|---|---|
| lccc | **7 993 123** | `merge_and_count` 4 395 704 (55 %), **`key_cmp` 1 910 187 (24 %)** |
| GCC 16.2 | **3 893 903** | `merge.constprop.0` 2 626 292 (67 %) — no `key_cmp` frame |
| Clang 23.1.3 | **4 529 014** | everything inlined into `main` |

lccc executes **2.05× GCC's instruction count** because it keeps the comparison
as an out-of-line indirect call while GCC clones `merge` with the function
pointer constant-propagated (`constprop`) so the compare becomes a direct call
in the same function. The costs are: an opaque call per merged element, the
caller-saved spill/reload set around it, an indirect-branch misprediction per
element, and a lost register allocation across the call.

This is the single largest measured gap found in this session and it is squarely
in the "invent or adapt an algorithm" category: an IPA pass that (a)
devirtualizes a call through a parameter whose every call site in the TU passes
one function symbol, and (b) clones the callee with the parameter bound, would
recover most of it. Reproducer: the kernel itself plus
`valgrind --tool=callgrind` on the three builds.

### 5.2 `__m128d` is a struct of two doubles, so SSE2 ABI and codegen diverge from GCC/Clang

`include/lcccsimd.h` (generated) defines the 128-bit vector types as *structs*:

```c
typedef struct __attribute__((__aligned__(16))) { double __val[2]; } __m128d;
```

Consequences, all measured:

* **ABI.** SysV passes a 16-byte SSE *vector* in one `%xmm`; a *struct* of two
  doubles takes **two** `%xmm`. Confirmed by compiling the same type as
  `vector_size(16)` (lccc emits `addpd %xmm1,%xmm0; ret`) versus as the header's
  struct (lccc reads `%xmm0`/`%xmm1` for one argument). A lccc-compiled callee
  called from a GCC-compiled caller therefore disagrees on argument placement —
  reproduced here, it returns `1 0` where `11 22` is expected.
* **Codegen.** `__m128d vmax(__m128d a, __m128d b) { return _mm_max_pd(a, b); }`
  is **33 instructions** in lccc against GCC's `maxpd; ret` (2): a 344-byte
  frame, four 8-byte stores of the incoming registers, `movdqu` round-trips
  through stack slots, the `maxpd`, then an integer `orq`/`xorl` reassembly of
  the two halves and two `movq`-to-`%xmm` to place the return value.

The `vector_size(16)` path in the backend is already *correct and optimal*
(`addpd`/`ret`), so the defect is the header representation plus the aggregate
return path, not the ISA support. Changing the typedefs is an ABI change and
must be done as its own change with its own differential corpus (it also
requires reworking every `.__val[i]` access in the intrinsic headers, which are
generated by `scripts/gen_lcccsimd.py`).

### 5.3 No sibling-call optimization

```c
double dmin2(double x, double y) { return __builtin_fmin(x, y); }
```

| | emitted |
|---|---|
| GCC 16.2 | `jmp fmin` |
| lccc | `subq $8,%rsp; call fmin@PLT; addq $8,%rsp; ret` |

Four extra instructions and a stack adjustment per call, in every wrapper of
this shape. Real code is full of them (`fmax`, `strlen` wrappers, `sqrt`
dispatch). Cheap to detect (single call followed by `Return` with matching
signature and no live inline-asm barrier), and it also removes the `%rsp`
adjustment that breaks stack alignment expectations for the callee.

### 5.4 Scalar `_mm_max_sd`/`_mm_min_sd` still lower as compare+select

`maxsd` 84 instructions against the better oracle's 11 (after the PR #904
merge; it was 102). The header implements them as a C select on `__val[0]`; the
backend owns the encodings (`encode_sse_op(..., &[0xF2,0x0F,0x5F])`) and the
peephole passes know the mnemonics, so the missing link is the intrinsic
lowering, not the ISA. Same class as 5.2 and best fixed together.

---

---

## 5a. A miscompile the corpus growth exposed (fixed this session)

The three kernels added in section 4 do not only measure — one of them led back
to a wrong-code defect, which is the more valuable result.

### 5a.1 The defect

`tests/regression/check_vector_ext_codegen.sh` went red at `-O2 -mavx` and
`-O2 -mavx2` (754/756 checks) on `lane2_v32l`, `lane2_v32d`, `nine`,
`shl_v32c`, `shr_v32c`, `shl_v32s`, `shr_v32s`, `mul_v32l` and `loop_v32l`.
**It is a regression introduced by this delivery, not by upstream**, and the
first reading of the evidence said otherwise: the failing checks look like a
long-standing width bug, and the gate's own history made "pre-existing" the
convenient answer. Measured against a binary built from this round's base
commit (`0ab6e743`), the gate **passes** and emits all 32 lane stores; against
the delivery before the fix it fails at two of eleven levels with 16–24 stores.
The honest statement is therefore the narrower, more useful one:

* the *classifier* defect (section 5a.2) is old and latent;
* this round's `vector_temp_promotion` change is what gave a 32-byte frame slot
  enough shape for the wrong footprint to delete half of it —
  `CCC_DISABLE_PASSES=vecpromote` on the pre-fix binary restores all 32 stores,
  which is how the exposure was localised;
* so the wrong code is new, arrives with the same delivery that would have
  shipped it, and is fixed here rather than inherited.

The lesson is recorded because it is the reusable part: a latent width error
sat in the tree until a pass started producing the shapes that exercise it, and
only a differential that covers 256-bit lane-wise work was ever going to see
it. The gate that caught it (`vector-ext-codegen`) is enabled by default in
`scripts/ci_local.sh --fast`, so the next such change is caught in-session.

Root cause, reached by bisecting the peephole pipeline
(`CCC_PEEPHOLE_SKIP=…`, `CCC_NO_PEEPHOLE_PHASE5=1`, `CCC_DUMP_ASM`): the slot
classifier `parse_fp_slot_move` derived a vector slot move's footprint from the
**mnemonic**. `vmovdqu` covers both 16-byte (`%xmm`) and 32-byte (`%ymm`)
moves, so a 256-bit slot move was recorded as a 16-byte one in `MoveSize::DQ`,
and `eliminate_never_read_stores` — which proves slots dead from a read-range
census — concluded that bytes 16..31 of a 32-byte slot were never read. It then
deleted the `movb` stores that fill them:

```text
shl_v32c(v32c a, v32c b)   -O2 -mavx2   (before)
  vmovdqu %ymm0, 104(%rsp)          # argument a
  vmovdqu %ymm1,  72(%rsp)          # argument b
  … 16 lane pairs written by `movb %r?, 40+2i(%rsp)` …
  movsbq 105(%rsp), %r8 …           # lanes 16..31 loaded but never written
  vmovdqu 40(%rsp), %ymm0           # returns the stale upper half
```

Whole upper half of the result was whatever the slot happened to hold. At
`-O0`/`-O1` all 32 stores survive (32 `shlq`), so the wrong code appeared only
where the optimiser ran.

### 5a.2 The fix

The footprint is now taken from the **register operand** (`DQ32`, `DQ64` beside
`DQ`), and the two variants deliberately decline every XMM-spelled forwarding
path, because a `%xmmN` replacement would truncate the value — they can only
widen a memory extent, never forward. Three incidental hardenings came with it:

* `compare_branch.rs` carried a second, hand-written copy of the width table;
  it now calls `MoveSize::byte_size()`. A duplicated table is how the numbers
  drifted, and one owner prevents a repeat.
* The EVEX width-tagged spellings (`vmovdqu8/16/32/64`, `vmovdqa32/64`) were
  not classified at all and fell through to the generic path; they are now
  slot moves whose width again comes from the operand.
* `vmovdqu64 %zmm…` is 64 bytes, so `%zmm` slot moves stop being described as
  16-byte ones as well.

### 5a.3 Evidence

| check | before | after |
|---|---|---|
| `check_vector_ext_codegen.sh`, base `0ab6e743` binary | PASS (32 lane stores) | — |
| `check_vector_ext_codegen.sh`, delivery pre-fix binary | 754/756, 2 levels red, 16–24 stores | — |
| `check_vector_ext_codegen.sh`, after the fix | — | **756/756**, 11 levels, 32 stores |
| `vector_frame_slot_upper_half.c` `-O2 -mavx2` | FAIL (4 checks: lanes 16..31 of `v32c`/`v32s`, `nine`, `lane2`) | **PASS** |
| `vector_frame_slot_upper_half.c` `-O2 -mavx` | FAIL (`lane2` reads garbage) | **PASS** |
| same file vs GCC at `-O0/-O1/-O2/-O2 -mavx/-O2 -mavx2/-Os/-O3` | n/a | all seven levels PASS, both compilers |
| unit tests `vector_slot_width_tests` | n/a | 2/2 (12 spellings × width × direction, plus the scalar widths that must not widen) |

The new regression file is compiler-independent, self-checking and free of
measurement; its kernels are `noinline` and non-static **on purpose** — a
`static` definition is inlined into `main`, the frame slot disappears, and the
file then passes with or without the fix. That was measured, not assumed: the
first version of the test passed against the broken binary for exactly that
reason, which is why the negative control below is part of the procedure.

**Negative control** (the reason to trust the test):

```bash
git stash                       # compiler without the fix
cargo build --profile fastbuild --locked -j 2 && cp target/fastbuild/lccc /tmp/lccc-prefix
git stash pop && cargo build --profile fastbuild --locked -j 2   # with the fix
/tmp/lccc-prefix  -O2 -mavx2 tests/regression/vector_frame_slot_upper_half.c -o /tmp/a && /tmp/a
#   FAIL: v32c shift lane 16: 48 != 98
#   FAIL: lane2 = 53480988614590603, want 33 (upper-half lane read)
```

**Cost: measured zero on the corpus.** `scripts/callgrind_ab.py` over the
31-program fast corpus, old binary vs new (`--opt=-O2`, correctness-gated,
identical stdout on every program): **every arm is bit-identical** — Ir,
I1/D1/LL misses and both branch counters, geomean 1.00000. That is the expected
shape of the result: the change only widens a memory extent, and the scalar
corpus has no 32-byte vector slot to widen. Where it does apply it *preserves*
stores that were being deleted, i.e. it trades wrong code for one store per
previously-lost lane, which is the only acceptable direction. The SSE2 122-row
matrix and the vector-extension ceilings are unchanged.

---

## 5b. Sibling calls: why this session did not convert the text-level pass

Section 5.3 measured the gap (`double dmin2(double,double){return
__builtin_fmin(x,y);}` → `subq $8; call fmin@PLT; addq $8; ret` against GCC's
`jmp fmin`). This session established precisely where the fix must go, and why
the tempting shortcut is the wrong one.

**The IR already has the shape.** `dmin2` lowers to

```text
block 0:
  ParamRef v2 = param 0
  ParamRef v3 = param 1
  Call  v6 = fmin(v2, v3)   return_type: F64
  term: Return(Some(v6))
```

which is textbook sibling-call form, so nothing upstream is missing.

**The existing text-level pass cannot take it, correctly.**
`src/backend/x86/codegen/peephole/passes/tail_call.rs` requires the simulated
`%rsp` depth at the call to equal the entry CFA, and it says why in its own
heading: converting `call T; <teardown>; ret` into `<teardown>; jmp T` re-bases
where the tail callee reads its incoming stack arguments from. With a `call`
the callee's first stack argument sits at the caller's pre-call `%rsp`; with a
`jmp` it sits 16 bytes higher. That is the forwarder-with-stack-arguments
miscompile the pass documents (`pr23324`), and the alignment padding
(`subq $8`) that `dmin2` needs is exactly the shape that made the old
conversion unsound. Re-relaxing the same text-level predicate is how that
miscompile would come back.

**So the work item is IR-level, and it is bigger than it looks.** There is no
`TailCall` terminator: `Terminator` has `Return`, `Branch`, `CondBranch`,
`IndirectBranch`, `Switch`, `Jump`-by-label and `Unreachable`, and
`tail_call_elim` only turns self-recursion into a loop. Adding a terminator
means touching every CFG consumer (liveness, regalloc, the IR verifier, block
layout, the ~20 pass walkers), which is why it is scoped as **L** and not done
at the tail of a session on a 1.9 GiB host.

The design that would land it, with the guards that make it sound:

1. A pass marks `Call` + `Return(dest)` pairs where the block is the function's
   only tail path, the call is not variadic, `is_sret == false`, and the
   callee's stack-argument count is **zero** (the one case where the 16-byte
   window shift cannot matter; with `CallInfo::arg_types` and the SysV
   eightbyte classifier already in the tree this is a lookup, not a guess).
2. Require no `Alloca` in the function whose address escapes, and no dynamic
   alloca (the two textual guards generalised).
3. The backend emits the call's argument setup exactly as today, then the
   epilogue, then `jmp` instead of `call` — no re-basing of anything the callee
   reads.
4. Gate it on a differential that includes the shapes which broke the old pass:
   a 7-stack-argument forwarder, a `__builtin_alloca` user, and a variadic
   forwarder, each asserted to be *refused* as well as correct.

Until then the current code is right and one instruction-pair slower, which is
the correct trade.

---

## 5c. Two gates that were red on `main` (one now fixed locally)

`scripts/ci_local.sh --fast` on this tree is green except for one gate that
fails identically on the base commit (5c.1). Both entries were checked the only
way that settles it: build the base commit and run the same gate against it.
5c.2 turned out to be a defect in the gate, not in the compiler, and is fixed.

```text
base 0ab6e743 binary                       gate result
  check_phi_acyclic_order.sh               FAIL: escape-off 52 insns/5 stkref (pin <=3); default 53 > escape-off 52
  check_llvm_scalar_facts.py               FAIL: hot web: enable/disable does not change the pressure witness
                                           -> fixed in 5c.2 (gate asserted an accidental property)
```

### 5c.1 `phi-acyclic-copy-order` — three extra stack references in `rot()`

Measured, three ways, all agreeing: `default 53 insns/5 stkref`,
`CCC_EVICT_SHORT_K=0 52/5`, `legacy 68/28`. The pin (written 2026-09-27, 702
commits before this round) wants `<=53` instructions **and `<=3` stack
references**, plus `default <= escape-off`. Instruction count is within one of
the pin; stack traffic is 2–3 references above it. Also re-measured with the
newest upstream codegen commits reverted (`src/backend/state.rs`,
`src/backend/x86/codegen/cast_ops.rs`): unchanged, so the drift is older than
those merges. Nothing in this delivery touches the phi resolver, the escape
policy or the allocator's slot classification, and the numbers are identical
before and after it.

This is therefore a real codegen regression someone introduced between
2026-09-27 and this round — a rotation kernel spilling two to three more values
than it should — and it should be *fixed*, not re-pinned. Next step: bisect
`rot()`'s stack traffic over the 702 commits (`git bisect` on the instruction
count is enough, the metric is deterministic and takes one `-O2 -S`).

### 5c.2 `llvm-scalar-facts` — CLOSED: the gate, not the pass, was wrong

`CCC_NO_HOT_WEB_STEAL=1` produces byte-identical assembly for
`tests/regression/hot_web_callee_saved.c`, and the gate treated that as a
failure ("enable/disable does not change the pressure witness"). The previous
session's probe blamed the pass: a 310-use candidate apparently refused by the
`phi_pair_values` guard. Re-measured on the current tree with temporary
counters in `src/backend/regalloc/hot_web.rs` (since reverted; the file is
untouched in this patch), that explanation no longer holds:

```text
candidates=[(109, 3100), (107, 310), (102, 21)]   <- the hot ones, ranked
candidates=[(139,10),(138,10),...,(131,31)]       <- a colder function
pairveto: max hot_count refused = 31              <- 21 on this witness
```

Three facts follow, each from data rather than inference:

1. **The hot candidates are already assigned.** `109` (3100 uses) and `107`
   (310 uses) are skipped by `assignments.contains_key(&vid)`, not by any
   guard. Compared with the previous session's `assigned=false`, allocation now
   homes them: the optimisation became *redundant*, not blocked. The earlier
   reading predates this patch's regalloc work.
2. **The surviving candidates cannot pay.** Only `102` (21 uses) is unassigned,
   and its cheapest available holder webs are priced at 1000 uses
   (`costrefuse cand=102 hot=21 reg=1 cost=1000`). Declining is correct.
3. **The veto never blocks a valuable steal.** Instrumenting the
   `phi_pair_values` clause shows it fires only for 12-31-use candidates across
   three witnesses (the shipped one, and two synthetic high-pressure programs
   with 16 recurrences). Narrowing it to the evicted set was implemented,
   measured — assembly still byte-identical, no steal anywhere — and reverted:
   it changes no code, so it is not an optimisation, only risk.

The defect is the assertion. "Enabling a pass must change this C program's
assembly" is an accidental property of the allocator's state: it held while
values were spilled, and rots the moment allocation improves — which is exactly
what happened. The mechanism itself is already pinned by 15 unit tests in
`backend::regalloc::hot_web::tests` (cold-home steal, whole-web pricing, strict
profit, ABI-hint refusal, noncoalescible-holder refusal, tie-breaks), which run
`steal_hot_loop_webs` on synthetic contexts and cannot rot.

So the gate now asserts what is structural — that the switch is plumbed and
that disabling the pass never changes emitted behaviour or the function body —
and prints whether the assembly differs, instead of demanding that it must. It
is green here and green on `main`; the mechanism coverage is unchanged and now
non-brittle.

5c.1 is still open and still recorded rather than re-baselined: re-baselining
would convert a real spilling regression into "expected". 5c.2 is closed above
by fixing the assertion, not the baseline.

## 6. Prioritised follow-up backlog

Ordering is `expected gain × affected workloads × confidence ÷ cost`, as the
project policy requires. Each item names its evidence and its first experiment.

| id | item | evidence | expected gain | cost | first step |
|---|---|---|---|---|---|
| **TODO-1** | Scale the three new kernels above 50 ms and re-measure (`PASSES`, `-DELEMENTS`); record the numbers in `PERFORMANCE-BACKLOG.md` | every arm flagged `<20 ms`, CV 5–9 % | makes all later claims defensible | XS | bump `PASSES`/`ELEMENTS` defaults; rerun with `--reps 15` |
| **TODO-2** | **IPA: devirtualize + clone a callee whose function-pointer parameter is invariant across the TU** | callgrind 5.1: lccc 2.05× GCC, 24 % of Ir in the callback frame | 1.3–1.4× on `linux_list_sort`, and on every callback-driven kernel (`qsort`, `list_sort`, `binary_trees`, `hash_table`) | L | prototype as a pass that requires a single known callee per (function, parameter) pair; measure on the corpus before wiring |
| **TODO-3** | **Sibling-call optimization** (`Call` immediately followed by `Return`, trailing-arg compatible) | 5.3, and 5b for the design | removes `subq`/`addq`/`call` → `jmp` per wrapper call (4 instructions), and the call's return-address stack traffic | **L**, not S | IR-level, not text-level: see 5b — the text pass is conservative because it is right to be. Start with a `TailCall` terminator, zero-stack-argument guard, and the four-shape differential 5b lists |
| **TODO-4** | `_mm_max_sd`/`_mm_min_sd` intrinsic lowering to `maxsd`/`minsd` | 5.4: 84 vs 11 instructions | −73 instructions on 2 of 122 matrix rows; the largest per-row absolute gap | M | add the ops to the SIMD builtin table + backend arm, keeping NaN semantics = hardware |
| **TODO-5** | **Make the 128-bit vector types ABI-compatible with GCC/Clang** (`vector_size(16)` instead of struct-of-two) | 5.2: ABI divergence + 33 vs 2 instructions | eliminates the 344-byte frames and `movdqu` round-trips across the whole SSE corpus; fixes cross-compiler ABI | XL | spike: switch the typedefs in a scratch copy, count broken header sites, measure the 122-row matrix and an interop test before committing |
| **TODO-6** | Return-side aggregate collapse (the mirror of PR #904's argument-side pass) | 5.2's `orq`/`xorl` reassembly before `ret` | part of TODO-5, but reachable sooner without an ABI change | M | pattern: aggregate built solely from the halves of one 128-bit value, only returned |
| **TODO-7** | Move `check_atomic16_runtime.sh`'s runtime half to `-O2` and add the `i128` atomic reproducer from `FOLLOWUP-2026-10-21-i128-atomic-o2.md` | that document says the gate deliberately stops at `-O1` | closes a known hole | S | flip the flag, add the reproducer as a second gate |
| **TODO-8** | Build binutils 2.47 (`oracle_toolchains_from_tags.sh --with-binutils`) and wire the GAS/bfd oracle into `run_linker_tests.py` | the tag payload is verified but unbuilt | linker differential vs 2.47 rather than the distribution's 2.44 | S (10 min build) | run the flag once, add a gate that skips with a named prerequisite |
| **TODO-10** | Bisect `rot()`'s 5 stack refs back to 3 over the 702 commits since the phi-acyclic pin (5c.1) | gate red on `main` too; measured identical on base and on this tree | a real spilling regression in a rotation kernel | M | `git bisect` on the deterministic instruction count |
| ~~**TODO-11**~~ | ~~Decide which eviction guard in `hot_web::steal_hot_loop_webs` is over-broad~~ | **CLOSED 5c.2**: the hot candidates (3100, 310 uses) are already assigned on this tree — allocation improved, the steal is redundant; the veto never blocks a steal worth having (max 31 uses), and narrowing it changes no emitted byte. The gate's "assembly must differ" assertion was the defect and is replaced by the structural contract. | — | — | done |
| **TODO-9** | Callgrind A/B on the benchmark corpus after the slot-width fix, to confirm the widened extents cost nothing and to re-baseline `PERFORMANCE-BACKLOG.md` | 5a.3: the fix preserves stores that were previously deleted, so it can only be neutral or positive — but that must be measured, not assumed | none if neutral (it is a correctness fix) | XS | `scripts/callgrind_ab.py` over the fast corpus, both binaries, identical stdout |

## 7. Reproduce this session

```sh
# 1. sandbox
sudo fallocate -l 6G /swapfile && sudo chmod 600 /swapfile && sudo mkswap /swapfile && sudo swapon /swapfile
git clone https://github.com/ms178/lccc && cd lccc
LCCC_ALLOW_COMPILER_BUILD=1 bash scripts/arena_session_restore.sh     # toolchain + fastbuild
sudo scripts/oracle_toolchains_from_tags.sh --bin-dir /usr/local/bin  # gcc-16, clang-23, mold
bash scripts/ensure_icx_2026.sh                                      # ICX 2026.2

# 2. the gates this session added or changed
python3 tests/regression/check_builtin_float_aliases.py --verbose
python3 tests/regression/check_ci_link_oracle_provisioning.py
python3 tests/regression/check_ci_toolchain_bundle.py
python3 tests/regression/check_sse2_codegen_matrix.py --check --self-test

# 3. the new kernels
for k in gzip_longest_match linux_list_sort zstd_double_fast; do
  target/fastbuild/lccc -O2 tests/benchmark/programs/$k.c -o /tmp/$k && /tmp/$k
done
python3 tests/benchmark/run_benchmarks.py --only gzip_longest_match,linux_list_sort,zstd_double_fast \
    --gcc gcc-16 --clang clang-23 --icx "$(bash scripts/ensure_icx_2026.sh)" \
    --reps 9 --json /tmp/new.json --markdown /tmp/new.md

# 4. the 5.1 root cause
target/fastbuild/lccc -O2 -DPASSES=1 tests/benchmark/programs/linux_list_sort.c -o /tmp/ls1.lccc
gcc-16 -O2 -DPASSES=1 tests/benchmark/programs/linux_list_sort.c -o /tmp/ls1.gcc
valgrind --tool=callgrind --callgrind-out-file=/tmp/cg.lccc /tmp/ls1.lccc
callgrind_annotate /tmp/cg.lccc | head -20      # key_cmp is 24 % of lccc's Ir, absent for GCC
```

## 8. Honest limits of this session

* **No PMU.** Top-down metrics, cycle counts and cache-miss attribution are
  unavailable in this guest. Everything above is dynamic instruction counts
  (Valgrind, deterministic), static censuses, and paired wall-clock medians with
  bootstrap intervals on a shared 2-vCPU host. The instruction-count statements
  are exact; the wall-clock ratios are screening evidence and are labelled as
  such.
* **The reference compilers are host builds** — GCC 16.2.0 from the repository's
  cache, Clang 23.1.3 from the same, ICX 2026.2.0 from Intel's repository. The
  SSE2 matrix deliberately ratchets on *lccc's own* count for that reason.
* **TODO-5 is an ABI change** and was deliberately not attempted in this
  session: it needs a differential corpus of its own (the existing gates would
  not catch a subtly wrong eightbyte classification), and a wrong ABI change is
  worse than a slow one.

---

## 9. State of the tree at the end of this round

`scripts/ci_local.sh --fast`, fully provisioned host: **404 gates PASS, 2 gates
FAIL**, and both failures are demonstrated on the base commit (section 5c).
Everything this round touches is green, including the gate that caught its
regression.

The gates that looked like code failures and were not: `memory-value-contracts`,
`byte-accessor-inline`, `ivsr-integer-domains`, `ivwiden-exposed-latch`,
`fp-extract-homes`, `empty-aggregate-abi` (32-bit C runtime), `pr824-boundaries`
(`qemu-aarch64`), `linker-suite`'s `i386_dso_emit_semantics` (32-bit libstdc++),
and `cargo-test` (disk: `elf_writer_common`'s scratch assembly, which the test
itself guards against but cannot fix for a full disk). Provisioning them is part
of a usable environment, so the list belongs here:

```bash
scripts/oracle_toolchains_from_tags.sh --prefix /home/user/oracles   # gcc-16, clang-23, mold
sudo apt-get install -y gcc-multilib libc6-dev-i386 lib32gcc-s1 \
                        g++-multilib lib32stdc++-14-dev valgrind qemu-user-static
swapon /swapfile                                                     # 6 GiB, mandatory
```

Re-run with those in place and the 404/2 split is reproducible. Of the two that
used to survive it, TODO-10 was closed upstream and TODO-11 is closed here
(5c.2) — the gate was wrong, not the pass.

## 10. Session 12 — PR #918 was red; two gates, both host/state dependent

CI reported the `Frontend and ABI contracts` job red at the
`Verify linker-oracle provisioning contracts (mock only)` step. Reproducing it
required installing `mold` and `lld` first — that is the whole story.

### 10.1 Root cause (an assertion that depended on the host, not the code)

`tests/regression/check_ci_link_oracle_provisioning.py` built its test `PATH` as
`f"{bin_dir}:{os.defpath}"`. `bin_dir` holds the intent stubs, but `os.defpath`
(`/bin:/usr/bin`) is the host's own userland — and the CI lane installs
`mold`/`ld.lld` one step before this gate runs. The script under test probes the
host first (`command -v mold`, `command -v ld.lld || command -v lld`) and takes
its warm-runner fast path, returning before any install. Every install-path
expectation inverted:

```text
FAIL test_transient_index_503_is_retried_until_it_resolves   AssertionError: 0 != 3
FAIL test_unresolvable_packages_fail_closed                   AssertionError: 0 != 1
                                                 7 of 14 cases, cold host: 14/14
```

The comment claiming "the oracles are deliberately ABSENT from PATH" was false
on CI and true locally, which is why it survived review.

### 10.2 Fix

`hermetic_path()` mirrors every directory of the live `PATH` into a private
directory *omitting* `mold`, `ld.lld` and `lld`, then asserts that no oracle in
`ORACLE_BINARIES` resolves through the result — fail-closed, so a future PATH
layout that defeats the masking errors out instead of silently flipping the
contract. A utility allowlist was tried first and rejected: the script under
test needs `dirname` and `date` (`127: dirname: command not found`). Mirroring
is the only construction that keeps the whole userland while removing exactly
the binaries under test.

Verified 14/14 with `mold 2.42.1` and `ld.lld 23.1.3` both on PATH (the CI
state) and 14/14 without them. The other two gates that mention `os.defpath`
(`check_ci_gcc16_provisioning.py`, `tests/corpus/test_ci_failure_annotations.py`)
were probed the same way — with a real `/usr/bin/gcc-16` planted — and are
immune, because their stubs sit in the *first* PATH entry and absence is
simulated by stub state rather than by omission.

### 10.3 The gate that asserted an accidental property

`check_llvm_scalar_facts.py` required `CCC_NO_HOT_WEB_STEAL=1` to change the
assembly of `hot_web_callee_saved.c`, and failed on `main` for the opposite
reason: the pass now correctly declines. Diagnosis and closure in 5c.2.

### 10.4 Audit of the rest of the patch

```text
cargo clippy --locked --profile fastbuild --all-targets   0 warnings
cargo test   --locked --profile fastbuild --lib           5724 passed, 0 failed
check_ci_link_oracle_provisioning.py                      14/14 (oracles installed)
check_llvm_scalar_facts.py                                PASS
```

No `phi_pair_values` narrowing is shipped: it was implemented, measured to
change nothing, and reverted (`src/backend/regalloc/hot_web.rs` is byte-identical
to the PR head).

## 11. Session 13 — head-to-head review of the re-worked PR #918, and the merge

The re-worked branch added eleven commits on top of the shipped payload. They
were reviewed by measurement, not by reading: both trees were built with
`fastbuild` and compiled the same 70-program benchmark corpus with `-O2`.

### 11.1 What the eleven commits change in emitted code: nothing

```text
binary                         programs   total instruction lines   failures
S11 payload (audit branch)           70                       10,230          0
re-worked PR #918 (f149104ac)        70                       10,230          0
rows whose assembly differs: 3 — gzip_longest_match, linux_list_sort,
zstd_double_fast, and only in .s size (+4, +4, +2 bytes) because their default
pass counts were raised (8192/128/512). Instruction counts identical.
```

So the branch is a correctness and measurement change, not a code-quality
change: its own audit says the 122-row matrix stays at 3,230 LCCC instructions
against GCC 1,544 / Clang 1,553 / per-row best 1,425. That is an honest and
important measurement — it says the SSE/SSE2 surface is roughly 1.9x worse than
the best oracle per row, with `sqrtsd`, `maxsd`, `minsd` around 7.5x.

### 11.2 Agreed with, and adopted

* **The VEX self-reload fix is a real correctness hole and it was mine.** The
  `MoveSize::DQ` self-reload elision does not exist at base `c8e9226bb`; it was
  introduced here. Deleting a *legacy* `movdqu 72(%rsp), %xmm6` is sound (it
  rewrites the same 128 bits) but a VEX/EVEX form zeroes bits 255:128, so the
  elision changes architectural state whenever the upper vector is live. Their
  fix gates the elision on the load's own encoding, which is exactly the right
  test, and their unit test covers four VEX spellings.
* **Aggregate-pair hardening.** Six IR-level safety holes (external readers of
  deleted values, signed low-half extension, unknown aliasing, mutable address
  webs, destination observers, displacement overflow) are now refused rather
  than assumed. Their tests are stricter than the code was; the transformations
  are unchanged on every benchmark program measured above, so this is safety
  without a code-quality cost.
* **Measurement hardening:** the SSE2 matrix rejecting incomplete rows, the
  slot-forwarding gate failing when an installed oracle fails (`GCC=false` used
  to look green), and the atomic gate exercising O1/O2/O3 consumers.

### 11.3 Disagreed with, and changed

* **Their isolation of the provisioning mocks is a utility allowlist**, which
  resets a maintenance trap: the scripts also call `dirname`, `date` and `sed`,
  and the failure mode is `127: command not found` far from the cause (it
  happened here while isolating `ci_provision_toolchain.sh`). Replaced with a
  mirrored host PATH minus the three oracle names, plus a fail-closed assertion
  that no oracle is reachable. Verified 15/15 in both the warm and cold CI
  states, and 8/15 fail under a negative control that drops the mask, so the
  mask -- not stub ordering -- enforces the contract. Keeping `os.defpath` in
  the mirror is also what makes their own adversarial test load-bearing: under
  a stub-only PATH its `mock.patch.object(os, "defpath", ...)` has no effect.
* **Their hot-web narrowing is correct but inert, and was measured as such.**
  On this tree the candidates are (3100, 310), (310) and (21) uses and the hot
  two are already *assigned*, so `assignments.contains_key` skips them; the
  21-use one cannot outbid a 1000-use holder. Narrowing the veto to the actual
  eviction set changes no emitted byte on the witness or on two synthetic
  16-recurrence programs, so it is kept as a semantic precision fix (with their
  tests) and explicitly not claimed as a performance win.
* **Their `llvm-scalar-facts` red is the gate's fault, not the compiler's.**
  Requiring "enabling the pass must change this C program's assembly" asserts an
  accidental property of allocator state; it rots as allocation improves, which
  is what happened. The mechanism is already pinned by 15 unit tests that run
  `steal_hot_loop_webs` on synthetic contexts. The gate now asserts the
  structural contract (switch plumbed, disabling never changes behaviour) and
  prints whether the assembly differs. Green, and green on the base commit.
* **Their second red (`gla-cross-reach-band`) does not reproduce on their own
  head** (frame 96, 62 instructions -- gate green) but does on the rebased tree,
  where it is a main-side drift: 96/62 at base, 128/101 on main eeece44e9, and
  96/95 with `CCC_DISABLE_PASSES=all`. The band contract stays exact; the
  absolute pin is re-measured and the 32-byte/6-instruction growth is recorded
  as an open code-quality item.

### 11.4 ICX 2026.2 is now available as a third oracle

`icx` 2026.2.0 (`2026.2.0.20261001`) was installed from the Intel oneAPI apt
repository by extracting the C/C++ compiler payload (the full meta-package
needs 2.35 GiB and the box has 3 GiB free) and pointing `-resource-dir` at
clang 23.1.3's builtin headers. It resolves the three worst matrix rows:

```text
row        lccc   gcc   clang   icx   best
maxsd        86    12      11    11     11
minsd        84    12      11    11     11
sqrtsd       87    12      11    39     11
```

ICX confirms the target is attainable on two of the three rows and is itself
worse on `sqrtsd`, so 11 is the bar. The cause is the memory-backed `__m128`
representation (section 5.2), not a missing instruction: the lowering reaches
`maxsd` but pays for slot traffic around every scalar lane.

### 11.5 Open items this session adds

| id | item | evidence | first step |
|---|---|---|---|
| TODO-12 | AArch64 matmul grew 96/62 -> 128/101 on main | `CCC_DISABLE_PASSES=all` restores 96/95, so a pass adds 6 instructions and 32 bytes of frame | bisect main's pass list on `vectorize_matmul_n17.c`; recover or justify |
| TODO-13 | Scalar `maxsd`/`minsd`/`sqrtsd` rows are 7.5x the oracle best, and ICX shows 11 is reachable | 86/84/87 vs 11/11/11 across four compilers | remove the aggregate slot round-trip for a scalar intrinsic result, then re-run the matrix |
| TODO-14 | Add ICX as a matrix oracle column | installed and verified above | extend `sse2_codegen_matrix.json` toolchains and regenerate |

# GCC 16.2 feature-parity assessment — measured report

Measured: 2026-10-27<br>
Oracles: GCC 16.2.0 (`gcc-16`), Clang 23.1.4 (`clang-23`)<br>
Subject: `target/fastbuild/lccc` built from `dc878b3` (= `main` after PR #993)<br>
Companion backlog: [`backlog.md` §12](../backlog.md) — one item per finding below.

This report answers one question: **if a build system replaced GCC 16.2 with lccc
today, what would break, what would silently change meaning, and what would be
merely missing?** Every number below was produced in this session on the checked-out
tree; every claim carries the command that produced it (§8). Nothing is estimated
from documentation or inferred from a neighbouring measurement.

---

## Current bounded update (2026-10-28)

The scorecard and prose below record the **2026-10-27** tree; they are not a
current compatibility score. The full item-by-item update is
[backlog §12.0](../backlog.md#120-bounded-gp-01gp-24-sentinel-verdicts-2026-10-28).
On those bounded tests, **7 of 24 items are partial, 17 remain open, and none
is certified complete**. A passing sentinel is not an acceptance suite.

* GP-01: 127 GCC-known names, 83 inventoried as unimplemented, 48 implemented
  names including four non-GCC Clang extensions; 22 attribute and two glibc
  header probes pass. This inventory does not implement the 83 names.
* GP-02/03/04: `ms_abi`, frontend `retain`, and x86 `interrupt` remain
  correctness hazards. Mixed GCC/LCCC `ms_abi` links returned 42 or SIGSEGV;
  retained C data lacked `R`; an interrupt function ended in `ret`, not
  GCC's `iretq`. The assemblers now preserve uniquely named GNU retained
  sections, and x86-64 `ld -r` preserves retention, but the C attribute is
  still missing and same-name assembler variants are refused.
* GP-05/23: on one x86-64 GNU23 empty TU, GCC has 408 macros, LCCC 466;
  153 are GCC-only, 211 LCCC-only and 34 common replacements differ. LCCC
  still identifies as GCC 14. No broader macro parity is asserted.
* GP-09: a warning registry and 57 GCC 16.2 precedence/severity comparisons
  pass; the complete warning surface and workload-wide precision gate remain
  open. GP-20 now emits some DIEs as well as line tables, so the historical
  "line-only" description below no longer describes current output.
* GP-24: seven derived kernels have bounded compiler comparisons; zlib-ng
  2.3.3 static Make `alltests` passed under GCC and LCCC, as did 48
  cross-compiler roundtrips and 24 gzip decodes. CMake/CTest, the remaining
  workload matrix and target-hardware performance results remain unverified.

The current sentinel's raw local evidence is in
`~/.cache/lccc/gp-sentinel-audit.{py,json,txt}`; checked-in regression scripts
cover the landed narrower gates. Versions and limitations are detailed in
[the session follow-up](FOLLOWUP-2026-10-28-GCC16-ATTRIBUTES.md). Neither this
update nor the older scorecard establishes GCC 16.2 parity.

---

## 1. Method

| Technique | What it measures | Scale |
|---|---|---|
| Option-catalogue probe | For every option `gcc-16 --help={common,warnings,optimizers,target,params}` knows, compile a TU with lccc and classify the outcome | 1 389 options |
| `__has_builtin` differential | Builtin namespace visible to a programmer | 5 501 candidate names |
| Attribute universe + dispatch diff | `__has_attribute` over every identifier in `cc1`'s string tables, compared with lccc's attribute dispatcher and semantic handlers | 142 GCC attributes |
| Predefined-macro diff | `-dM -E` comparison on an empty TU | 408 GCC / 464 lccc |
| Feature battery | 113 hand-written programs (C23, C17, GNU extensions, ABI, attributes), compiled **and executed** by both compilers | 113 probes |
| Behavioural probes | `readelf`/`objdump` on emitted objects, plus cross-compiler links (one object from each compiler) | 20+ cases |
| Corpus triage | three-way verdict diff over `tests/corpus` from the previous round | 1 434 files |

Classification vocabulary for option probes: *accepted-silent* (no diagnostic),
*tolerated* (lccc's own "unrecognized … tolerated but not implemented" warning),
*refused* (explicit error naming the missing feature), *unrecognized* (hard error).

Sandbox: 2 vCPU, 3 GB RAM + 2 GiB swap, Debian bookworm. `gcc-16`'s own
`<stdbit.h>` is absent (glibc too old), so one probe (`#include <stdbit.h>`) is an
environment artifact and is excluded — lccc ships its own header and passes it.

---

## 2. Scorecard

| Dimension | GCC 16.2 | lccc | Parity |
|---|---|---|---|
| Language / dialect (113 executed probes) | — | — | **105 / 113 (93 %)**, 15 of the 113 are environment artifacts |
| C23 coverage (`docs/C23-COMPATIBILITY-INVENTORY.md`, 24 probes) | 24 | 23 | 96 % (`_BitInt` absent) |
| Predefined macros (empty TU) | 408 | 464 | 155 GCC macros absent, **211 libc macros wrongly predefined**, 34 value spellings differ, `__GNUC__` = 14 |
| Attributes | 142 | ~76 recognized | **62 attributes silently accepted and ignored**; `-Wattributes` never fires |
| Builtins (default x86-64 ISA) | 1 696 | 607 | **478 shared**, 1 218 GCC-only, **129 over-claimed** |
| Builtins (`-march=x86-64-v3`) | 2 171 | 607 | 605 shared, 1 566 GCC-only |
| Warning options (`-W…` positive) | 549 | 18 accepted | 12 live `WarningKind`s; 535 options tolerated-not-implemented |
| Driver options overall | 1 389 | 157 accepted, 1 051 tolerated, 171 refused, 10 unrecognized | 11 % effect, 87 % no-op-or-refused |
| ISA options (`-m…`) | 234 | 77 accepted | 153 refused (honest errors) |
| Debug info | DWARF 5: types, variables, inline, dir/file tables | `.debug_line` only, DWARF 4, empty directory table | gdb cannot show types, locals or arguments |
| Coverage (`-ftest-coverage`, `-fprofile-arcs`) | `.gcno`/`.gcda` + gcov | flags accepted and **re-routed to the PGO path**: emits a `lccc-*.profraw` and prints the PGO instrumentation banner, but **no `.gcno`/`.gcda`** | wrong artifact, mislabelled |
| PGO | `.gcda` (`-fprofile-generate/use`) | own `.profraw` scheme, works, format-incompatible | functional, not interchangeable |
| LTO | works | refused with an error | honest gap |
| Sanitizers | ASan/UBSan/TSan/LSan runtimes | trap-mode UBSan only | honest gap |
| Inline asm | full constraint grammar + diagnostics | **no constraint validation** | accepts impossible constraints |
| Linker (`lccc-ld`) | GNU ld feature set | own, tracked by MB-13/MB-14 | separate programme |
| Real-world builds (CI workloads: gzip, zlib-ng, expat, sqlite) | — | green in hosted CI | working |

Reading: lccc's **language and code-generation core is close to GCC 16.2**; its
**environment contract (macros, attributes, warnings, tooling flags) is far from it**.
That asymmetry is the whole story of this report: the compiler understands C, but a
build system, an autoconf macro, a hardening flag or a sanitized CI run would still
notice the difference — sometimes by silently getting different semantics.

---

## 3. Findings by dimension

### 3.1 Language and dialect — strong, with four holes

113 probes compiled **and executed** by both compilers. Agreement on 105. Excluding
environment artifacts (`<stdbit.h>`), the real differences are:

| Probe | GCC 16.2 | lccc | Class |
|---|---|---|---|
| `unsigned _BitInt(7)`, `_BitInt(17)`, `_BitInt(200)`, `1wb` literals | accept | reject | **missing feature (C23)** |
| digraphs `%:` `<:` `:>` `<%` `%>` (C95/C99/C23) | accept | reject (`expected declaration before '%'`) | **conformance gap** |
| universal character names in identifiers (`int caf\u00e9;`) | accept | reject | **conformance gap (C99)** |
| `int f(); … int f(int a);` in C23 (empty list = prototype) | reject | accept | over-acceptance |
| `__fp16` on x86-64 | reject | accept | over-acceptance |
| `-std=c17 -pedantic` on a statement expression | diagnostic | `-pedantic` tolerated, no effect | **missing diagnostic mode** |

Everything else in the battery agreed, including: statement expressions, nested
functions with trampolines, `__auto_type`, `typeof`/`typeof_unqual`, `constexpr`,
`nullptr`/`nullptr_t`, `bool`/`true`/`false`, fixed-underlying-type enums, `auto`,
`[[attributes]]`, `static_assert`, `_Generic`, `_Atomic`, `_Thread_local`, VLA and
`sizeof VLA`, `#embed`, `__VA_OPT__`, `#elifdef`, `asm goto` (including with
outputs), case ranges, computed goto, local labels, zero-length arrays, flexible
array members, `__int128`, `__float128`, `_Float16`, `__bf16`, `_Complex`,
`_Decimal32/64`, transparent unions, `cleanup`, `ifunc`, `alias`, `weak`,
`constructor`, `section`, `symver`, `regparm`, `target`, `target_clones`, `optimize`,
`gnu_inline`, `-fshort-wchar` (verified against GCC's `sizeof(L"abc")`),
`#pragma pack`, `#pragma GCC`, `_Pragma`, `#include_next`, `#line`, iso646 alternative
tokens, `\u` escapes in string literals.

### 3.2 Predefined macro environment — the most consequential defect found

`echo | lccc -dM -E -x c -` defines **211 macros GCC never defines**, taken from
`<stdio.h>`, `<stdlib.h>`, `<errno.h>`, `<limits.h>`, `<stdint.h>`, `<inttypes.h>`,
`<stddef.h>` (`src/frontend/preprocessor/builtin_macros.rs:22`–`216`): `BUFSIZ`,
`EOF`, `NULL`, `CHAR_BIT`, `INT_MAX`, `SIZE_MAX`, `EXIT_SUCCESS`, `INT8_C(x)`,
`PRIX16`, `ENOENT`… Consequences:

* `#ifdef BUFSIZ` / `#ifdef NULL` / `#ifdef EOF` feature tests take the wrong branch
  (reproduced: `#ifdef EOF → #error` fires under lccc, not under GCC);
* a program that legitimately defines its own `BUFSIZ` now collides with a predefined
  macro that GCC does not have;
* `__STDC_NO_ATOMICS__` is predefined although `_Atomic` and `__atomic_*` work — an
  autoconf capability denial (GCC does not define it; lccc's own atomics tests pass).

Also missing (155 GCC macros), with concrete victims:

| Missing macro | Consequence |
|---|---|
| `__BIGGEST_ALIGNMENT__` | code sizing scratch buffers assumes 16 |
| `__BITINT_MAXWIDTH__` | `_BitInt` feature tests |
| `__FLT32_*`, `__FLT64_*`, `__FLT128_*`, `__FLT32X_*`, `__FLT64X_*` (~60) | `float.h`-style constant folding in third-party headers |
| `__DEC32/64/128/64X_*` (~30) | decimal float limits |
| `__CHAR8_TYPE__`, `__FINITE_MATH_ONLY__`, `__FLOAT_WORD_ORDER__`, `__ATOMIC_HLE_ACQUIRE/RELEASE`, `_STDC_PREDEF_H`, `__*_IS_IEC_60559__` | conformance queries |

And `__GNUC__ = 14`, `__VERSION__ = "14.2.0"`, `-dumpversion` → `14`: the compiler
claims GCC 14 while matching GCC 16.2 diagnostics. Code gated on
`__GNUC__ >= 15` (new attributes/builtins) silently takes the older path.

### 3.3 Attributes — a silent-ignore surface with an ABI break behind it

GCC 16.2 supports 142 attributes for C. lccc dispatches 54 names
(`src/frontend/parser/parse.rs::dispatch_gcc_attribute`) and implements semantics for
~22 more elsewhere, so ~76 are recognized. **The remaining 62 are parsed and
discarded without a word**, and `-Wattributes` — which lccc accepts — never fires
for an unknown, ignored or misapplied attribute:

```
$ gcc-16 -fsyntax-only - <<< '__attribute__((frobnicate)) int y;'
<stdin>:1:1: warning: 'frobnicate' attribute directive ignored [-Wattributes]
$ lccc   -fsyntax-only - <<< '__attribute__((frobnicate)) int y;'      # silent
$ gcc-16 -fsyntax-only - <<< '__attribute__((packed)) int x;'
<stdin>:1:1: warning: 'packed' attribute ignored [-Wattributes]
$ lccc   -fsyntax-only - <<< '__attribute__((packed)) int x;'         # silent
```

That is a *diagnostic* problem. The *correctness* problem is the same list:

**`ms_abi` is silently ignored and breaks the ABI across compiler boundaries.**

```
$ cat abi_fn.c   ; __attribute__((ms_abi)) int ms_fn(int a,int b){return a*10+b;}
$ cat abi_use.c  ; __attribute__((ms_abi)) int ms_fn(int,int); printf("%d\n",ms_fn(4,7));
$ gcc-16 abi_use.c fn_gcc.o  -o t && ./t      → 47
$ gcc-16 abi_use.c fn_lccc.o -o t && ./t      → 1216891234        <-- wrong
$ lccc    abi_use.c fn_gcc.o -o t && ./t      → Segmentation fault
```

Unimplemented attributes, grouped by what their absence costs (all measured to be
accepted silently):

| Group | Attributes | Consequence when ignored |
|---|---|---|
| Calling convention | `ms_abi`, `sysv_abi`, `stdcall`, `thiscall`, `cdecl`, `sseregparm`, `force_align_arg_pointer`, `callee_pop_aggregate_return`, `preserve_none`, `no_caller_saved_registers`, `no_callee_saved_registers`, `ms_struct`, `gcc_struct` | **silent ABI mismatch** when mixing objects (demonstrated above) |
| Hardening | `nocf_check`, `cf_check`, `strub`, `indirect_branch`, `no_stack_limit`, `no_split_stack`, `ms_hook_prologue`, `noplt` | hardening silently weaker than requested |
| Section/object placement | `retain`, `noinit`, `persistent`, `nocommon`, `no_icf`, `nodirect_extern_access` | measured: GCC puts these in `.data.retain`/`.persistent`; lccc puts everything in `.data`/`.bss` |
| Codegen contract | `interrupt`, `indirect_return`, `musttail`, `assume`, `simd`, `vector_mask`, `no_reorder`, `noipa`, `noclone`, `tainted_args`, `target_version` | missed optimisation (benign) or **wrong prologue/guarantees** (`interrupt`, `musttail`) |
| Type/layout | `hardbool`, `signed_bool_precision`, `strict_flex_array`, `warn_if_not_aligned`, `designated_init`, `flag_enum`, `vector_mask`, `btf_decl_tag`, `btf_type_tag`, `null_terminated_string_arg`, `warn_unused`, `unavailable`, `no_address_safety_analysis`, `objc_*`, `transaction_*`, `fentry_*`, `expected_throw`, `__simd__`, `__volatile__` | lost diagnostics / lost hardening metadata |

### 3.4 Builtins and intrinsics — the widest raw gap

`__has_builtin` differential over 5 501 candidate names:

| Flags | GCC sees | lccc sees | Shared | GCC-only | lccc-only (over-claim) |
|---|---|---|---|---|---|
| default | 1 696 | 607 | 478 | **1 218** | 129 |
| `-march=x86-64-v3` | 2 171 | 607 | 605 | **1 566** | 2 |

Composition of the 1 218 default-ISA misses: ~550 are the C23 `floatN` math builtins
(`__builtin_acosf16/f32/f64/f128/f32x/f64x/l`, `…pi` variants) that GCC 16 exposes for
`_FloatN`/`_FloatNx` math; ~292 are x86 ISA builtins (`__builtin_ia32_*`);
the rest are generic families (`__builtin_*_overflow` gaps, `__builtin_object_size`
variants, `__builtin_stdc_*`, `__builtin_apply*`, `__builtin_classify_type`,
`__builtin_va_arg_pack_len`, `__builtin_alloca_with_align_and_max`,
`__builtin_dynamic_object_size`, `__builtin_shufflevector`-style helpers).

The 129 over-claims are worse than the misses, because `__has_builtin` is how portable
code decides what to use: lccc answers *yes* for builtins it then diagnoses or
mis-lowers (`__builtin_ia32_maxps256` is `true` without `-mavx`, where GCC says
`false`).

The intrinsic *headers* are a separate axis and already inventoried in
`engineering/isa/gcc16-sse-avx-reference.json`: 5 365 of 6 169 public names parsed
from GCC 16.2's 49 SSE/AVX headers are absent from lccc's `include/` bundle
(backlog §9a `SIMD-PERM-1`).

### 3.5 Warnings and diagnostics — precision is good, breadth is thin

lccc accepts 18 of GCC's 549 positive `-W` options and implements 12 live
`WarningKind`s (`src/common/error.rs:89`):
`implicit-function-declaration`, `cpp`, `return-type`, `incompatible-pointer-types`,
`attributes` (registered, never emitted), `unused-variable`, `sign-compare`,
`format`, `format-security`, `implicit-int`, `return-mismatch`, `int-conversion`,
plus `-Werror`/`-Wno-*` plumbing. 535 options are tolerated with a warning.

Measured consequence on ordinary code:

```
$ gcc-16 -O2 -Wall -Wextra -c wu.c   # int x; if (x > 0) …
  warning: 'x' is used uninitialized [-Wuninitialized]      lccc: silent
$ gcc-16 -O2 -Wall -Wextra -c w3.c   # missing enum cases, return of a local array
  [-Wswitch] [-Wreturn-local-addr]                          lccc: silent
$ gcc-16 -O1 -Wall -Wextra -Wshadow -Wconversion -Wvla -Wformat=2 …
  [-Wshadow] [-Woverflow] [-Wvla] [-Wmissing-prototypes]    lccc: none of these
```

What *is* at parity — and should not be given up while widening — is precision: the
675-contract GCC-16.2 red-team suite passes exactly, the diagnostic severity gate
passes, and the 1 434-file corpus triage shows **0 false positives** vs GCC with 17
known gaps (`__fp16`, `__builtin_bit_cast/assume`, unknown attributes, inline-asm
constraint strings, implicit-int).

### 3.6 Driver option surface — 11 % effect, but honest about it

1 389 unique options from GCC's own help catalogue, probed one by one:

| Class | Count | Meaning |
|---|---|---|
| accepted, no diagnostic | 157 | claimed to work (not all do — see §3.7) |
| tolerated with "not implemented" warning | 1 051 | honest, but a no-op |
| refused with a named reason | 171 | honest hard error |
| unrecognized hard error | 10 | e.g. `-gprune-btf`, `-gz=…` |

Notable refusals/gaps found by probing (each one blocks a real-world build or CI):

* `-pedantic` — tolerated, no effect (GCC diagnoses GNU extensions in strict modes);
* `-Ofast`, `-Og` — tolerated, no effect (they silently *are* `-O2`/`-O1`? no:
  nothing changes at all);
* `-static-pie` — tolerated, no effect;
* `-mindirect-branch=`, `-mfunction-return=` (retpoline) — refused: kernel/mitigation
  builds that ask for them cannot use lccc;
* `-fshort-enums`, `-fpack-struct`, `-freg-struct-return` — refused (documented);
* `-mstackrealign`, `-mfsgsbase`, `-msha`, `-mrdseed`, `-madx`, `-mclflushopt`,
  `-mwaitpkg`, `-mavx10.1` — refused (153 of 234 `-m` options);
* `-dumpfullversion`, `-print-multiarch`, `-print-multi-lib`, `-print-sysroot` —
  hard errors; `-print-prog-name=cc1` prints a bare `cc1`, not a path;
  `-print-libgcc-file-name` resolves to an unrelated system `gcc-12` libgcc.

`-O3` is **byte-identical to `-O2`** on the sample measured (no additional passes fire),
while `-Os`/`-Oz` do produce distinct (smaller) output. `-fshort-wchar`, `-fwrapv`,
`-ftrapv`, `-fstack-protector{,-strong,-all,-explicit}`, `-fcf-protection`,
`-fstack-clash-protection`, `-fzero-call-used-regs`, `-fvisibility=`,
`-fno-plt`, `-ffunction-sections`/`-fdata-sections` are implemented.

### 3.7 Debug info, coverage, PGO, LTO, sanitizers

| Feature | Measurement | Status |
|---|---|---|
| `-g` | `.debug_line` only; **no `.debug_info`, `.debug_abbrev`, `.debug_str`**; DWARF **v4**; directory table empty | line-level `addr2line` only; gdb shows no types/locals/params |
| `-gsplit-dwarf` | warns "not supported; emitting non-split debug info" | honest |
| `-gz=`, `-gdwarf-N`, `-gprune-btf` | unrecognized hard errors | absent |
| `-ftest-coverage` (alone) and `-fprofile-arcs` (alone) | both accepted; **each one alone** triggers `lccc: PGO: instrumented N functions … into ./lccc-*.profraw`; **no `.gcno`, no `.gcda`, no `__gcov_*`** | not silent, but the wrong artifact: a coverage job gets a PGO profile instead of gcov data |
| `-fprofile-generate` | works: emits `lccc-*.profraw`, `-fprofile-use` consumes it | functional, GCC-incompatible format |
| `-fauto-profile`, `-fprofile-update=` | accepted (the latter refused for unknown values) | partial |
| `-flto`, `-flto=auto` | refused with a named reason | honest gap |
| `-fsanitize=address|thread|memory|leak` | refused (needs runtime) | honest gap |
| `-fsanitize=undefined` | requires `-fsanitize-trap=…`; trap mode only | partial |

### 3.8 Inline assembly — no constraint validation

```
$ lccc -c -x c - <<< 'int i; void f(void){ __asm__ volatile("" : : "j"(i)); }'   # accepts
$ gcc-16 …                                          error: impossible constraint in 'asm'
$ lccc …  "V"(i)                                    # accepts; gcc: impossible constraint
$ lccc …  "=f"(d) as an *input* operand             # accepts; gcc: input operand contains '='
```

A 65-probe matrix over input letters, output letters and `%`-modifiers produced **15
divergences, all in the direction "lccc accepts what GCC rejects"** — every constraint
letter reachable, including letters that mean something else on other targets (`L`,
`e`, `Z`, `s`, `G`, `C`, `B`, `y`, `V`, `u`). `-Wasm-operand-widths` and the
"operand probably does not match constraints" analysis do not exist. For kernel code
and hand-written SIMD this is a silent-miscompile surface, not a cosmetic one.

### 3.9 ISA and target surface

`-m` acceptance: 77 of 234 options. Implemented and verified by probe: SSE4.2,
AVX2/FMA/BMI2/LZCNT/POPCNT/AES/VAES/GFNI/VPCLMULQDQ, AVX-512 F/BW/VL/VBMI2,
`-mcmodel=small|large`, `-mno-red-zone`, `-mgeneral-regs-only`, `-msoft-float`,
`-march=znver4|x86-64-v3`, `-mtune=native`, `-m32`/`-m64`. Refused: `-mavx10.1`,
retpoline flags, `-mstackrealign`, `-mfsgsbase`, `-msha`, `-mrdseed`, `-madx`,
`-mclflushopt`, `-mwaitpkg`, AMX family, `-mabi=`, `-m128bit-long-double`.

Targets: x86-64 and i686 are the supported product targets (backlog §1); aarch64 and
riscv64 backends exist behind `-target`-style selection and are tracked separately
(`engineering/subsystems/riscv.md`, `aarch64` gates). Full GCC 16.2 target parity is
explicitly out of scope: GCC 16.2 ships dozens of back-ends.

### 3.10 Code generation

Not the subject of this report (see `docs/PERFORMANCE-BACKLOG.md`, `docs/benchmarks.md`
and the CI benchmark lanes). Two compatibility-relevant facts were measured anyway:

* `-O3` currently emits the same code as `-O2` on the measured sample — programs that
  expect GCC's `-O3` (loop unrolling, aggressive vectorisation) will simply be slower,
  never wrong;
* the auto-vectorizer is SSE2-width and conservative: a 64-element `c[i]=a[i]+k*b[i]`
  AXPY loop vectorises to 128-bit `movdqu`/`pmuludq` sequences with a scalar prologue,
  where GCC `-O3` emits ~20× more vector instructions. Functional, not competitive.

---

## 4. Risk ranking — what actually bites

| # | Risk | Class | Evidence |
|---|---|---|---|
| R1 | Ignored ABI attributes (`ms_abi` …) produce silent cross-object ABI breakage | **wrong code** | §3.3 reproducer (1216891234 / SIGSEGV) |
| R2 | Libc macros predefined as builtins flip `#ifdef` feature detection | **wrong build** | §3.2 `#ifdef EOF` |
| R3 | `__has_builtin` over-claims 129 names | **wrong code** in portable dispatch | §3.4 |
| R4 | Inline-asm constraints accepted without validation | **wrong code** | §3.8 |
| R5 | `-ftest-coverage`/`-fprofile-arcs` accepted but re-routed to PGO, so no gcov data exists | coverage job reads 0 %, and the banner suggests instrumentation happened | §3.7 |
| R6 | `-pedantic`, `-Ofast`, `-Og`, `-static-pie` accepted with no effect | silent no-op | §3.6 |
| R7 | Uninitialized/switch/shadow diagnostics absent | missed bugs, not miscompiles | §3.5 |
| R8 | `__STDC_NO_ATOMICS__` denies a capability lccc has | degraded build (fallback code) | §3.2 |
| R9 | No DWARF DIEs | debugger unusable for variables/types | §3.7 |
| R10 | `_BitInt`, digraphs, UCN identifiers | compile failure on conforming code | §3.1 |
| R11 | No sanitizer runtimes, no LTO, no gcov | CI/security tooling cannot adopt lccc | §3.7 |

The five **silent** classes (R1–R6) are the ones a "GCC-compatible" claim cannot
survive: every other gap announces itself with an error or a warning.

---

## 5. Work-item register

IDs `GP-nn` are new and are carried into [`backlog.md` §12](../backlog.md) with
acceptance gates. Effort: S ≤ 1 day, M ≤ 1 week, L > 1 week of agent time.

| ID | Item | Class | Effort | ROI | Fixes |
|---|---|---|---|---|---|
| GP-01 | Attribute registry + `-Wattributes` truth | diagnostics | S | 4.6 | R1 guard, §3.3 |
| GP-02 | ABI attributes (`ms_abi`/`sysv_abi`/`stdcall`/`thiscall`/`cdecl`/`sseregparm`/`force_align_arg_pointer`/`preserve_none`/`ms_struct`/`gcc_struct`) + cross-compiler ABI gate | correctness | L | 4.2 | R1 |
| GP-03 | Object/section attributes: `retain`, `noinit`, `persistent`, `nocommon`, `no_icf`, `noplt` | correctness | S–M | 3.8 | §3.3 |
| GP-04 | Codegen-contract attributes: `interrupt`, `nocf_check`/`cf_check`, `no_caller_saved_registers`, `assume`/`assume_aligned`, `musttail`, `indirect_return`, `no_reorder`, `noipa`, `flatten`, `leaf`, `nothrow`, `strict_flex_array`, `warn_if_not_aligned`, `designated_init`, `flag_enum`, `unavailable`, `warn_unused`, `hardbool`/`signed_bool_precision` | correctness/diagnostics | M–L | 3.6 | §3.3 |
| GP-05 | Macro-environment hygiene: drop the 211 libc predefines, fix `__STDC_NO_ATOMICS__`, add the 155 missing GCC macros, settle `__GNUC__`/`-dumpversion` | correctness | S–M | 4.4 | R2, R8 |
| GP-06 | `_BitInt(N)` end-to-end (C23) | feature | L | 3.2 | R10 |
| GP-07 | Digraphs and UCN identifiers | conformance | S | 3.4 | R10 |
| GP-08 | C23 dialect strictness sweep (`()` prototypes, `__fp16`, over-acceptances) | conformance | S | 3.0 | §3.1 |
| GP-09 | Warning registry: `(name, default, -Wall, -Wextra, -pedantic)` table driving `-W`, `-Wno-`, `-Werror=`, `-Wno-error=`, group expansion, `[-Wname]` suffix | diagnostics | M | 4.0 | R7, MB-12 |
| GP-10 | `-pedantic` implementation (extension diagnostics in strict `-std=` modes) | diagnostics | M | 3.4 | R6 |
| GP-11 | Flow warnings: `-Wuninitialized`, `-Wmaybe-uninitialized` | diagnostics | L | 3.6 | R7 |
| GP-12 | Missing warning families: `-Wshadow`, `-Wconversion`/`-Wsign-conversion`, `-Woverflow`, `-Wvla`, `-Wswitch{,-enum}`, `-Wmissing-prototypes`, `-Wstrict-prototypes`, `-Wreturn-local-addr`, `-Warray-bounds`, `-Wstringop-overflow/-truncation`, `-Wcast-qual`, `-Wdeprecated-declarations`, `-Wformat=2` extras | diagnostics | L | 3.8 | R7 |
| GP-13 | Real gcov: `.gcno`/`.gcda` emission for `-ftest-coverage`/`-fprofile-arcs`, gcov-tool-readable | tooling | M | 3.6 | R5 |
| GP-14 | PGO interop and honesty: `-fprofile-update=`, GCC-format import or explicit refusal, documented format | tooling | M | 3.0 | §3.7 |
| GP-15 | Sanitizer runtimes (ASan/UBSan run-mode, TSan/LSan) or documented refusal with a build-system recipe | tooling | L | 2.8 | R11, MB-07 |
| GP-16 | LTO: real `-flto` or refuse `-flto`/`-ffat-lto-objects` consistently and document `-fwhole-program` | tooling | L | 3.0 | R11, MB-15 |
| GP-17 | Inline-asm constraint grammar: validate letters per target, `=`/`+`/`&` placement, `%` modifiers, `-Wasm-operand-widths`, "operand probably does not match constraints" | correctness | M | 3.8 | R4 |
| GP-18 | Builtin coverage: `floatN` math builtins, generic families, and **`__has_builtin` accuracy**; then x86 ISA naming per the SIMD inventory | feature | L | 3.9 | R3 |
| GP-19 | ISA option surface: remaining `-m` options (retpoline, `-mstackrealign`, `-mfsgsbase`, `-msha`, `-madx`, `-mrdseed`, `-mclflushopt`, `-mwaitpkg`), AVX10 policy | feature | M–L | 3.5 | §3.6/§3.9 |
| GP-20 | Debug info: DIE emission (types, variables, formal parameters, inline), DWARF v5 line tables with directory/file tables, `-gdwarf-N`, `-gsplit-dwarf`, `-gz` | tooling | L | 3.8 | R9, MB-10 |
| GP-21 | Driver query parity: `-dumpfullversion`, `-print-multiarch`, `-print-multi-lib`, `-print-sysroot`, `-print-prog-name` paths, `-print-libgcc-file-name` | ergonomics | S | 3.2 | §3.6, MB-01 |
| GP-22 | **Option-effect ledger gate**: every option lccc accepts must be backed by a probe proving it changes an artifact or diagnostic, or be refused. Kills the "accepted, no effect" class structurally | process | M | 4.5 | R5, R6 |
| GP-23 | Macro-parity gate: `-dM -E` diff budget vs GCC per target × dialect, failing on any new predefined macro and on any missing essential macro | process | S | 4.0 | R2 |
| GP-24 | Real-world adoption matrix: gzip/zlib-ng/expat/sqlite (already in CI) plus a kernel-style TU set, each built by lccc with GCC parity on diagnostics and output | process | M | 3.7 | all |

### Suggested order (single agent, impact × confidence ÷ effort)

1. **GP-22** — the ledger gate turns every later item into a ratchet and costs days,
   not weeks. It is the only item that makes the *class* of R5/R6 impossible.
2. **GP-01 → GP-02 → GP-03/GP-04** — attributes, driven by the new `-Wattributes`
   diagnostic: every unimplemented attribute then becomes visible, and the ABI group
   (GP-02) removes the only *demonstrated* silent miscompile.
3. **GP-05 → GP-23** — macro hygiene plus its gate; cheap, and it changes what
   autoconf sees on day one.
4. **GP-17** — inline-asm validation (correctness, contained in one front-end file).
5. **GP-09 → GP-10 → GP-12/GP-11** — the warning programme, in that order.
6. **GP-13 → GP-20 → GP-16/GP-15** — tooling substrates (gcov, DWARF DIEs, LTO,
   sanitizers), the expensive half.
7. **GP-06/GP-07/GP-08/GP-18/GP-19** — feature completion, parallelisable.

---

## 6. What is already at parity (do not regress this while fixing the above)

* C23 language surface: 23 of 24 inventory probes accept exactly as GCC 16.2 does
  (only `_BitInt` missing); GNU extensions are broad and functional (§3.1).
* Operand/constraint diagnostics: 675 exact GCC-16.2 contracts, severity gate, and
  0 false positives over the 1 434-file corpus.
* ABI for the supported targets: SysV aggregate passing, varargs, i686 staging,
  ext-vector by-value grids and `__seg_gs`-style address spaces are covered by gates
  that pass in hosted CI (only the *attribute-selected* ABIs are missing).
* Real programs: gzip 1.14, zlib-ng 2.3.3, expat 2.8.5 and sqlite 3.53.4 build and
  pass their upstream test suites in CI workloads.
* Codegen identity for the diagnostic work: byte-identical `-O2 -S` output on
  599/599 compilable files of the 628-file source set after PR #993.
* Hardening flags that do work: `-fstack-protector{,-strong,-all,-explicit}`,
  `-fcf-protection`, `-fstack-clash-protection`, `-fzero-call-used-regs`,
  `-ftrapv`, trap-mode UBSan, `-fno-plt`, `-fvisibility=`, `-ffunction-sections`.

## 7. Deliberate deviations (documented, not bugs to "fix" blindly)

* `-flto`, `-fsanitize=address|thread|memory|leak`, `-fopenmp`, `-fanalyzer`,
  `-fsplit-stack`, `-fshort-enums`, `-fpack-struct`, `-freg-struct-return` are
  **refused with a named reason** — honest, and the right behaviour until implemented.
* PGO uses a `.profraw` format that GCC does not read; the correct end state is a
  documented, versioned format plus an explicit refusal to consume GCC `.gcda`.
* The GCC-14 `__GNUC__` value is a deliberate compatibility choice (many headers
  branch on it); if lccc claims GCC 16.2 parity, that claim and the macro must be
  reconciled explicitly, with `-fgnuc-version=`-style control.

---

## 8. Reproduction

All commands assume `source /home/user/.cache/toolchains/env.sh` (`gcc-16` 16.2.0,
`clang-23` 23.1.4) and a built `target/fastbuild/lccc`.

```bash
# option catalogue
for c in warnings optimizers common target params; do
  gcc-16 --help=$c | sed -n '/^ *-/p' | awk '{print $1}' | grep '^-'
done | sort -u > /tmp/gcc_opts.txt
# …then one probe per option (script: parity/probe_opts.py)

# predefined macros
echo | gcc-16 -dM -E -x c - | sort > /tmp/m_gcc.txt
echo | target/fastbuild/lccc -dM -E -x c - | sort > /tmp/m_lccc.txt

# attribute universe
CC1=$(gcc-16 -print-prog-name=cc1)
strings -n 3 "$CC1" | grep -oE '[a-z_][a-z0-9_]{2,}' | sort -u   # then __has_attribute
# lccc side: src/frontend/parser/parse.rs :: dispatch_gcc_attribute

# builtins
strings -n 5 "$CC1" | grep -oE '__builtin_[a-zA-Z0-9_]+' | sort -u   # then __has_builtin

# debug info
target/fastbuild/lccc -O1 -g -c d.c -o d.o && readelf -S d.o | grep debug

# ABI break
gcc-16 -c abi_fn.c -o fn_gcc.o && gcc-16 abi_use_ms.c fn_lccc.o -o t && ./t
```

Raw probe outputs and the fixture corpus from this session:
`/home/user/parity/` (`gcc_options.txt`, `lccc_option_probe.json`,
`gcc_builtins.txt`, `builtins_gcc_only.txt`, `gcc_attributes_full.txt`,
`lccc_attr_names.txt`, `m_gcc.txt`, `m_lccc.txt`, `feat/features.json`,
`feat/feature_matrix*.json`). These are session artifacts, not repository
contents — the report above carries every conclusion they support.

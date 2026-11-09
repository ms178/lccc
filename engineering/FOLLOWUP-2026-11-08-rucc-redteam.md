# RUCC red-team audit follow-up — 2026-11-08

## Scope and identities

- LCCC under audit: `ms178/lccc` main/base `dcd1822c4c9f07ed4284eb96d0347a6bf97e03c1`, branch `arena/0fc3839e-lccc`.
- Competitor: `tamnd/rucc` `4f9f5d857349ffd8ddfdcdbd79d6dadb87dc50e9` (Apache-2.0), shallow clone at `/home/user/rucc-review`.
- Toolchains: exclusively the hash-verified `ms178/lccc` `harness-cache-8` tag (`d66c5ac881e2168790b8e228dd548f025ab83e59`), restored by `scripts/restore_agent_toolchain_cache.py`; 12 files verified. Rust/Cargo 1.99.1, GCC 16.2.0, Clang 23.1.5, binutils 2.47.20260726, mold 3.0.0, GDB 18.1, QEMU user 11.1.3.
- Host: two vCPUs, 3.8 GiB RAM, exactly 4 GiB active `/swapfile`, Intel Xeon @ 2.60 GHz virtual host. **No physical i7-14700KF and no hardware PMU evidence.**
- LCCC build policy: `fastbuild`, Rust `-O1`, LTO off, incremental, 256 codegen units, debug=0, exactly `-j2`, cached Clang driver + mold. Baseline build completed in 3m46s; baseline binaries preserved in `/home/user/lccc-baseline-dcd1822/`.
- Rucc audit build: release profile overridden to `-O1`, LTO off, 256 codegen units, debug=0, incremental, `-j2`, mold; completed in 1m44s. crates.io is unreachable, so all 13 locked dependencies were replaced by GitHub source pins whose manifest versions match the lock exactly (`/home/user/rucc-deps/PINS.txt`). This is **not** the official release asset and **not** a crates.io checksum-identical build. The GitHub release asset route returned an HTML redirect outside the permitted hosts and was discarded.

## What was measured

Correctness screening ran the existing `tests/benchmark/run_benchmarks.py` corpus against LCCC, GCC 16.2, Clang 23.1.5 and the pinned rucc build. Twelve workload-derived kernels passed output equivalence for all four compilers: gzip CRC-32, zlib-ng Adler-32, Expat XML scan, Expat SipHash-2-4, SQLite varint, SQLite VDBE serial, Linux find-bit, Linux rbtree, glibc memcmp, glibc strstr, gzip longest-match and SHA-256 transform.

Timed screening used `-O2`, seven measured rounds, two excluded warm-ups, CPU0 pinning, randomized compiler order, retained MAD outliers, wall clock plus child CPU time, and no PMU probe. Raw report: `/home/user/rucc-timed.json` (676,555 bytes, SHA-256 recorded in `engineering/evidence/2026-11-08-rucc-audit/summary.json`); artifacts: `/home/user/rucc-timed-artifacts/`. The committed summary is deliberately below the 100 KiB bulk-evidence threshold.

Median wall times (ms) and paired LCCC/reference ratios from the runner:

| Workload | LCCC | GCC | Clang | rucc | LCCC/GCC | LCCC/Clang | LCCC/rucc |
|---|---:|---:|---:|---:|---:|---:|---:|
| gzip_crc32 | 153.62 | 169.21 | 170.33 | 169.27 | 0.904 | 0.897 | 0.902 |
| zlib_ng_adler32 | 40.15 | 39.81 | 37.73 | 41.98 | 1.014 | 1.063 | 0.956 |
| expat_xml_scan | 45.69 | 43.05 | 40.40 | 41.99 | 1.083 | 1.143 | 1.117 |
| expat_siphash24 | 264.70 | 301.66 | 280.66 | 333.96 | 0.869 | 0.981 | 0.784 |
| sqlite_varint | 29.61 | 27.33 | 28.16 | 27.76 | 1.084 | 1.062 | 1.021 |
| linux_find_bit | 16.47 | 15.39 | 15.10 | 15.02 | 1.067 | 1.091 | 1.114 |
| glibc_memcmp | 11.25 | 10.91 | 10.92 | 10.53 | 1.026 | 1.030 | 1.092 |
| sha256_transform | 296.12 | 313.39 | 349.43 | 420.54 | 0.995 | 0.931 | 0.812 |
| linux_rbtree | 16.86 | 16.66 | 16.44 | 16.32 | 0.981 | 1.001 | 1.044 |
| glibc_strstr | 4227.53 | 4234.68 | 4507.57 | 4522.51 | 0.998 | 0.935 | 0.932 |
| gzip_longest_match | 159.68 | 154.61 | 144.37 | 157.58 | 1.032 | 1.122 | 1.019 |
| sqlite_vdbe_serial | 21.57 | 17.77 | 17.51 | 18.27 | 1.221 | 1.233 | 1.189 |

Aggregate paired-median geometric means: LCCC/GCC **1.0192**, LCCC/Clang **1.0365**, LCCC/rucc **0.9911**. LCCC is faster than rucc on 5/12 and slower on 7/12; the near-unit aggregate is **not** a decisive overall win. The defensible conclusion is workload-dependent: LCCC is materially ahead on SHA-256 and SipHash ARX kernels and glibc strstr, while rucc/Clang/GCC are ahead on SQLite VDBE serial, Expat XML scan and Linux find-bit. Short cases (<20 ms) carry launch/scheduler noise; several runs have >5% CV and retained outliers. These are VM screening numbers, not Raptor Lake microarchitectural claims.

Static disassembly was retained per compiler/workload. Instruction counts are structural only, not dynamic counts. Examples: LCCC lowers Linux's generic `__ffs` decision tree to `bsf`, while rucc emits the branch/cmov tree; LCCC nevertheless loses the find-bit workload, so the gap is not explained by that lowering alone. LCCC's `scan_all_records` rematerializes the FNV prime inside the byte loop and keeps the outer record index in a stack slot; GCC/Clang/rucc differ in hoisting and register pressure. `gzip_longest_match` main is 385 LCCC instructions vs 320–324 for the references. These are leads, not yet validated optimizations.

## Concrete defect found and fixed

LCCC returned unconditional `0` for every `__has_feature` / `__has_extension` in both `#if` and ordinary code. GCC 16.2 and Clang 23.1.5 answer `1` for implemented C11/C23 capabilities, tls, asm-goto extensions and active UBSan rows; headers such as FreeBSD's `sys/cdefs.h` and Linux feature probes use these answers to select code paths. The blanket zero silently selects fallbacks.

The fix adds `src/frontend/preprocessor/feature_queries.rs`: a GCC-16.2-mode capability table gated on LCCC's *implemented* capabilities, wired through the driver-selected dialect, `-pedantic-errors` (not `-Werror=pedantic`), and active UBSan checks. It handles armoured spellings (`__c_atomic__`), macro aliases, `#if` and code-line contexts. Rows for unimplemented deprecated/unavailable/enumerator attribute diagnostics and absent address/hwaddress/leak/thread sanitizers deliberately remain `0`. Regression coverage is in `tests/regression/check_frontend_preprocessor_rules.sh` section 10 and Rust unit tests; the measured GCC/Clang matrix is retained in `engineering/evidence/2026-11-08-rucc-audit/feature-query-oracle.json`.

The capability-row approach was informed by rucc's Apache-2.0 GNU matrix and its truthfulness rule ("answering untruthfully is worse than answering no"). **No rucc source was copied.** LCCC is CC0; a future verbatim Apache-2.0 transplant must carry its own license/notice rather than being silently relicensed.

Also fixed the fastbuild wrapper's explicit job override: `LCCC_BUILD_JOBS=2` is no longer silently reduced by the low-RAM heuristic, and `ci_local.sh`/hosted Build now request it explicitly. The default unattended path remains memory-bounded; the research path uses the installed 4 GiB swap as an OOM backstop, not a speed claim.

## Rucc ideas worth keeping / not worth wholesale porting

- Worth adopting as discipline: capability queries must be truth-gated; rewrite rules should carry proof status and a solver-required CI lane; unverified/bounded-width rules must be inventoried separately. Rucc's verifier CI installs Z3 5.1.0, sets `RUCC_REQUIRE_SOLVER=1`, and checks rule directories against its own docs/UNVERIFIED.md inventory; nine rules are proved only at 4/8 bits. Its build scripts generate matchers but do not prove rules.
- LCCC already has extensive proof, differential, fuzz, peephole-invariance and benchmark-output infrastructure. Wholesale porting rucc's optimizer/e-graph/register allocator is not justified by this evidence and would be high-risk. Targeted transplants should be driven by a reproducer and an A/B measurement.
- Rucc's optional e-graph rewriter is not the default pipeline; default selection is driven by `-Zrewriter=` and the level pipeline. Do not describe rucc as "e-graph optimized by default" without tracing the selected arm.
- Rucc's safety monitor and litmus/replay harnesses are interesting correctness assets, but they depend on rucc IR/runtime conventions; port the *test shapes* first, not the monitor wholesale.

## Known gaps and limitations

- LCCC accepts but does not yet diagnose `deprecated("msg")`, `unavailable("msg")` and enumerator attributes in the probes run here; the corresponding feature rows stay `0` until sema implements them. `unavailable` use should be an error, not a silent accept.
- ICX/ICC, LLVM `ld.lld`, `clang++`, `qemu-x86_64` and a working `-m32` environment are absent from the verified cache/sandbox. Compiler Explorer failed with three TLS EOFs. No ICX/lld/CE ranking is claimed.
- mold 3.0.0 is a Rust/Cargo build; the historical CMake `MOLD_TARGETS` preset no longer exists. The x86-only feature set is `x86_64,i386,system-allocator`. Linker-oracle setup still does not reuse the verified cached mold and checks existence rather than version in places; this remains a prioritized script fix.
- Rucc's official release binary was not obtained; the audit binary is a pinned source build with GitHub dependency substitutions. Dependency tags match locked versions but are not crates.io archive checksums.
- No compiler miscompile was established in the twelve-kernel screening. Screening is not exhaustive correctness proof.
- Full `ms178/archpkgbuilds` project builds (gzip, zlib-ng, expat, SQLite, Linux, glibc) were not run; the existing workload-derived excerpts were used. Full-project extraction/build is remaining work.

## Prioritized remaining work

1. Validate the feature-query fix and job-override change with targeted tests, `ci_local.sh --fast`, and hosted PR CI; snapshot each validated tree into `/home/user/ms178-1.patch`.
2. Reduce the SQLite VDBE serial gap: reproduce the inner FNV byte loop, test constant hoist/rematerialization and outer-index register pressure, and require output-equivalence plus paired A/B timing before landing.
3. Reduce Linux find-bit overhead: investigate the unnecessary callee-saved `%r14` push/pop and indexed load folding in the zero-word loop; keep the existing `bsf` lowering.
4. Investigate Expat XML scan and gzip longest-match branch/layout and inlining differences against Clang/GCC.
5. Implement deprecated/unavailable/enumerator attribute diagnostics, then flip the truthful feature rows to `1` with differential tests.
6. Make linker-oracle setup reuse the hash-verified cached mold/binutils and assert exact preferred versions; keep the X86/i686 Cargo feature preset and record revisions.
7. Extend the corpus with pinned `ms178/archpkgbuilds` excerpts/full builds, especially zlib-ng, gzip, expat, SQLite, Linux and glibc, preserving provenance and license boundaries.
8. Re-run the timed corpus on the physical i7-14700KF with verified PMU counters before making any Raptor Lake or "beats GCC/Clang/ICX" claim.
9. Consider porting rucc's solver-required rule-proof CI shape and bounded-width inventory to LCCC's existing proof gates, only where it closes a demonstrated coverage gap.

## Reproduction pointers

- Cache restore: `python3 scripts/restore_agent_toolchain_cache.py --tag harness-cache-8` (lock: `engineering/toolchains/agent-cache-lock.json`). There is no `--restore` flag.
- LCCC build: `source /home/user/.cache/lccc/agent-toolchains/env.sh && LCCC_BUILD_JOBS=2 ./scripts/build_lccc_fast.sh`.
- Correctness/timing: `tests/benchmark/run_benchmarks.py --candidate rucc=/home/user/rucc-audit-bin/rucc-4f9f5d85 --compilers lccc,gcc,clang,rucc --reps 7 --warmup 2 --cpu 0 --no-pmu-probe ...`.
- Feature gate: `CCC=target/fastbuild/lccc bash tests/regression/check_frontend_preprocessor_rules.sh`.
- Local CI mirror: `./scripts/ci_local.sh --fast`; snapshot: `scripts/lccc-snapshot.sh` into `/home/user/ms178-1.patch`.

## 2026-11-08 continuation: defects found by red-teaming the first patch

The first feature-query revision built but **failed its own regression gate**.
Line-by-line audit of `resolve()` found a real design error, not a test typo:

1. **Extension answers were dialect-gated.** `resolve(name, extension=true, mode)`
   reused the *feature* availability expression, so `__has_extension(c_atomic)`
   answered 0 in C99 even though LCCC accepts `_Atomic` there as an extension
   (GCC 16.2 answers 1). The gates are now separate and documented: implemented
   here → available in this TU (sanitizer) → standard in this dialect. The
   dialect gate applies to the feature answer only; `-pedantic-errors` collapses
   the extension answer onto it. A linear `ROWS.iter().find()` was replaced by a
   `match` on `&str` (length switch + memcmp, no second name list to drift).
2. **Contiguous-fragment assertions were order-dependent.** Four "failures" were
   the test matching a fragment that spanned unrelated declarations. The gate now
   asserts each `key=value` independently and reports exactly which row diverged.
3. **Operand macro expansion was missing in `#if`.** Measured: GCC 16.2 expands
   the operand of `__has_feature`, `__has_extension`, `__has_builtin`,
   `__has_attribute` and `__has_c_attribute` (`#define FEAT c_atomic` ⇒ 1, chained
   macros and `gnu::deprecated` included); Clang does not for feature/builtin.
   LCCC's code-line path expanded the whole line first but its `#if` path resolved
   before expansion, so the two paths disagreed with each other and with GCC.
   Both now share `expand_query_identifier` / the attribute-token re-split.
4. **Malformed operands were silently accepted.** GCC and Clang both error.
   `__has_feature()`/`(123)`/`((x))`/missing `(`/junk after the name now produce
   GCC-worded errors and answer 0, and recovery consumes the *balanced* operand
   (literal-aware) so no stray `)` reaches the expression parser.
5. **Pre-existing `__has_builtin` gap:** GCC answers 1 for the unprefixed library
   spelling (`memcpy`, `strlen`); LCCC answered 0. Canonicalization now maps an
   unprefixed name through LCCC's own builtin registry, so the claim stays
   truthful (unknown names still answer 0) and the x86 target-flag gate is applied
   to the canonical name.
6. **UBSan truthfulness nuance:** LCCC implements UBSan in trap mode only and
   *rejects* `-fsanitize=undefined` without `-fsanitize-trap`; the first test
   revision asserted a query value for a mode the driver refuses. The gate now
   pins the rejection and replays sanitizer modes with the trap flag. The oracle
   differential asserts that `-fsanitize=address`/`kernel-address` are rejected
   rather than advertised.

### New data-driven gate

`tests/regression/check_feature_query_oracle.py` replays the committed GCC 16.2
matrix (21 names × 18 modes) against LCCC, requires GCC parity for implemented
rows, truthful zeros for the rest, and `#if`/code-line agreement: **1346 checks,
all passing**. It runs inside the existing `frontend-preprocessor-rules` gate, so
both CI mirrors execute it without new parity contracts. The shell gate adds the
full c89..gnu23 × ±`-pedantic-errors` matrix, `-Werror=pedantic`, user
`-D__STDC_VERSION__`, sanitizer modes, armour/half-armour, macro operands,
literals, and malformed-operand diagnostics.

### Linker oracle (follow-up item 6) — implemented

`scripts/ensure_mold_300_binary.sh` reuses the hash-verified agent-cache mold
3.0.0 (the same digest `ci_install_mold_oracle.sh` pins for the official upstream
release asset) and **proves** the x86 target set functionally: it links a trivial
object for `elf_i386` and `elf_x86_64` and checks ELF class/machine with readelf.
`--self-test` includes offline negative probes (missing binary, wrong version,
right version that cannot link) — all pass. Both `tools/linker/setup_oracles.sh`
and `tests/linker/setup_oracles.sh` now prefer that verified binary
(`MOLD_PREFER=cache`, seconds instead of ~40 min) and fall back to the source
build. The source preset was aligned to upstream v3.0.0 `cli/Cargo.toml`:
`--no-default-features --features x86_64,i386,system-allocator` (the `tools`
variant previously omitted `system-allocator`, i.e. it built a *different*
allocator oracle than the `tests` variant). `MOLD_PREFER=source` remains the
choice for link-timing research (`-C target-cpu=native`, system allocator).

### Harness wipe

The sandbox was recreated mid-session: everything outside the repo (toolchain
cache, swap, baseline binaries, raw rucc artifacts) was lost. The committed
evidence copies under `engineering/evidence/2026-11-08-rucc-audit/` survived and
are the reason the oracle matrix is still available. Swap was re-ensured at
exactly 4 GiB and the cache re-restored from `harness-cache-8` (12 files verified).
`/home/user/ms178-1.patch` is refreshed by `scripts/lccc-snapshot.sh` after each
validated landing.

## Deeper rucc audit (second pass, source re-cloned after the wipe)

The pinned rucc commit `4f9f5d85` is itself titled *"Answer __has_feature and
__has_extension as gcc 14 does (#4107)"* — the direct analogue of this PR. That
makes it the highest-value comparison available, so it was audited line by line.

### Independent confirmation of the corrected design

`crates/rucc-gnu/src/lib.rs` splits exactly the two gates the first LCCC revision
conflated:

* `Mode::has(when)` — availability. Returns `true` for `Always | C11 | C23`
  (availability is **dialect-free**) and consults the sanitizer flags otherwise.
* `Mode::is_feature(when)` — the dialect gate, applied to the feature answer only.
* `has_extension()` — under `-pedantic-errors` it delegates to `has_feature()`;
  otherwise it answers from the extension table, falling back to the feature row
  gated by `has()` (availability), **not** `is_feature()`.

That is behaviourally identical to the corrected LCCC `resolve()`, arrived at
independently (LCCC's was derived from the GCC 16.2 measurement matrix). Two
independent derivations agreeing is stronger evidence than either alone.
LCCC's lookup is a `match` on `&str`; rucc range-checks an interned symbol then
linearly scans eight operators — LCCC's dispatch is at least as cheap.

### Differences that are deliberate, not defects

| Aspect | rucc | LCCC | Why |
|---|---|---|---|
| Operator existence | gated on persona `__GNUC__ >= 14`; older personas leave them plain names so FreeBSD `sys/cdefs.h` can define them | always present | LCCC has a single GCC 16.2 persona and no `-fgnuc-version` (verified: GCC 16.2 itself rejects that flag — it is Clang's). **If LCCC ever grows persona selection, the operators must be gated at >= 14.** |
| `address_sanitizer` | 1 when `-fsanitize=address` is on | 0 always | LCCC has no ASan; the driver rejects the flag. Advertising it would be a false claim. |
| Row set | GCC's eleven names | 18 rows, including explicit `implemented: false` rows | LCCC documents its zeros in the table instead of omitting them, so a future implementer flips one flag rather than rediscovering the name. |
| Operand macro expansion | (not exercised by its matrix) | GCC 16.2 parity, both paths | Measured directly on gcc-16.2; Clang differs. |

### rucc's attribute discipline, checked against LCCC

`rucc-gnu`'s `Answer::{Warn, Error}` encodes a rule worth keeping: ignoring `hot`
costs speed, but ignoring `packed`, `aligned`, `section`, `no_sanitize` or `naked`
produces **wrong code**, so those must be refused rather than warned about.
LCCC was probed against GCC 16.2 on that contract:

* `struct S { char c; int i; } __attribute__((packed))` → `sizeof == 5` on both.
* `struct A { int x; } __attribute__((aligned(64)))` → `_Alignof == 64` on both.
* unknown attribute → `'…' attribute directive ignored [-Wattributes]` on both.

No defect found here; the discipline is already honoured.

### Pass inventory (99 rucc-opt modules vs 166 LCCC pass modules)

Named capabilities present in rucc with no same-named LCCC counterpart:
`ipasra` (IPA scalar replacement of aggregates), `ipvrp` (interprocedural value
range propagation), `header_copy` (loop header copying), `pressure` (explicit
register-pressure model), `memssa`, `gcm` (global code motion), `reload`.
LCCC covers much of the same ground under different names (`ipa_pta`, `ipcp`,
`live_range_shrinkage`, `web_congruence`, `expr_sink`, `licm`, `loop_preheader`,
`loop_rotate`, `load_forward`, `redundant_loads`) and has far broader bit-idiom
and vector coverage. **No wholesale port is justified**: rucc's e-graph is
opt-in, and nine of its rules are proved only at 4/8 bits (rucc docs/UNVERIFIED.md).

### The measured performance gap, and why the obvious fix would REGRESS

Regenerated the worst workload from the timed run (`sqlite_vdbe_serial`, LCCC
1.221x GCC). Static instruction counts at -O2: `scan_all_records` LCCC 99 / GCC 84
/ Clang 116; `vdbe_serial_get` LCCC 81 / GCC 87 / Clang 76.

The outer loop's induction variable lives in a **stack slot** in LCCC:

```
.LBB30:  addq $1, 16(%rsp)          # r++ in memory
         movq 16(%rsp), %rax        # reload for the exit test
         jb   .LBB23
.LBB23:  movq 16(%rsp), %r10        # reload AGAIN for the indexed access
         leaq rec_offsets(%rip), %r8
         movl (%r8,%r10,4), %r9d
```

GCC strength-reduces the same array walk to a marching pointer
(`movq $rec_offsets, 8(%rsp)` then `movq 8(%rsp), %rax; movl (%rax), %edx`), so it
never materialises the index. The obvious LCCC fix — let IVSR reduce this group —
is **already implemented and deliberately disabled**, with measurements recorded
in `src/passes/iv_strength_reduce.rs`:

* the `sib_scaled` skip fires because stride-4 indexing is a "free" scaled SIB;
* the comment above the derived-recurrence knob records that enabling it
  regressed five golden workloads (gzip_crc32 +6.2% insns, glibc_memcmp +6.4%
  insns/+23.5% stackmem, expat_xml_scan +23.5% stackmem, stencil5 +7.5%,
  zlib_ng_adler32 +6.2% stackmem — **CI RED on PR #602**) and an 11-rep
  interleaved A/B found runtime losses (expat_xml_scan +3.9..4.5%, glibc_strstr
  +4.3%, linux_find_bit +1.8%, linux_rbtree +1.2%) with **zero confirmed wins**;
* the stated root cause is the allocator's *latch phi-web parking*: every added
  loop-carried web materialises to a slot home at the latch. GCC keeps
  recurrences in callee-saved registers across calls. The file says to revisit
  the knob **after** the allocator work.

The independent measurement here reproduces exactly that blocker. **Conclusion:
the highest-value performance work is the register allocator's loop-carried
value placement, not IVSR.** Flipping the IVSR knob without the allocator fix is
known-regressing and must not be shipped. This is recorded rather than
"fixed" precisely because the data says the naive fix is a regression.

### Path-consistency defect found and fixed in this pass

`#define NOTIDENT c_generic_selections tls` + `__has_feature(NOTIDENT)`:
the code-line path diagnosed `expected ')' after 'c_generic_selections'` (GCC's
text, because it expands the line first) while the `#if` path diagnosed
`expected ')' after 'NOTIDENT'`. `expand_query_identifier` now returns the
expansion on failure so both paths quote GCC's first expanded token; a
regression asserts both paths print the identical diagnostic.


## Third red-team pass: query-operator lifetime and CI contract repairs

A differential probe against GCC 16.2 and Clang 23.1.5 found a second real
preprocessor defect in the first patch: `#undef __has_feature` (and the other
`__has_*` operators) was ignored. The operator stayed live in `#ifdef`, `#if`
and ordinary code, and a user `#define __has_feature(x) 7` was pre-empted by the
builtin in `#if` even though the code-line path honoured the macro. GCC/Clang
both withdraw the operator on `#undef`, let a user macro take over, and restore
the saved state with `push_macro`/`pop_macro`.

Fix: one `is_query_operator` list now drives existence, suppression and
evaluation. `MacroTable::suppressed` covers the operators as well as dynamic
builtins; `query_operator_active` is false while suppressed or while a user
macro owns the spelling. Both the `#if` and code-line resolvers consult it.
`#undef`/`#define` emit GCC's unclassified warnings (`-w` and plain `-Werror`
control them; `-Wno-cpp`/`-Wno-builtin-macro-redefined` do not), and
`-pedantic-errors` promotes live-builtin takeover to an error. A withdrawn
operator left in `#if` is now a hard "missing binary operator" error instead of
silently evaluating as the identifier alone; the expression parser rejects any
trailing tokens and empty conditions.

New gates: `check_query_operator_lifetime.py` (155 checks across 9 operators:
initial/undef/redef/push-pop/skipped-undef/alias/warning controls/pedantic) and
Rust units for suppression, snapshot restore and trailing-token rejection.
The preprocessor unit suite is now **116 passed / 0 failed**; the full frontend
gate remains green (1346 oracle checks + capability execution + 410 fuzz cases
+ 155 lifetime checks). Clippy `-D warnings` and rustfmt are clean.

CI repairs forced by the hosted red: the capability gate is now registered in
both CI mirrors (it was only reached transitively, which `check_ci_gate_parity`
correctly rejects as orphaned), and two `grep -q` early-exit consumers in the
frontend gate were converted to `grep -c >/dev/null` so a compiler crash cannot
be masked as SIGPIPE under `pipefail`. `check_pipefail_sigpipe.py` and
`check_ci_gate_parity.py` now pass locally.

### Operand-diagnostic parity for `__has_builtin` / `__has_attribute`

The same red-team pass found that malformed `__has_builtin` and
`__has_attribute`/`__has_c_attribute` operands were silent zeros (or left stray
tokens), while GCC 16.2 rejects them. Both resolvers now share the feature
query's diagnostic/recovery discipline: GCC-worded errors, balanced
literal-aware operand consumption, and macro expansion before judging the token
sequence. Attribute scope/name identifiers macro-expand (`SCOPE::deprecated`,
`gnu::NAME`, and a whole `SCOPED` token), matching GCC in both `#if` and code.
`__has_c_attribute` deliberately uses GCC's measured `__has_attribute`
diagnostic spelling. The lifetime gate grew to **181 checks**; the seeded fuzz
now also exercises withdrawn and user-owned operator states (**2880 cases across
three seeds**, zero panics/hangs/signals). The capability gate now asserts the
query answers are identical on the i686/AArch64/RISC-V sibling drivers, not just
x86-64. Preprocessor units remain green; Clippy `-D warnings` and rustfmt clean.

### Directive-only operators and a fuzz-found out-of-bounds panic

`__has_include`, `__has_include_next` and `__has_embed` in ordinary code were
left as plain identifiers (silent accept). GCC 16.2 diagnoses "'…' used outside
of preprocessing directive" but still evaluates the operand; LCCC now matches
that persona. Malformed `#if __has_include()` / missing `(` now emit GCC's
header-name diagnostics instead of silent zeros.

Extending the seeded fuzz to those operators immediately found a real panic:
`__has_embed("` / `__has_embed('` with an unterminated literal sliced
`bytes[start..len+1]` (`utils.rs:74`, range end out of range). The open-coded
balance loop incremented past `len` after `skip_literal_bytes` had already
returned `len`. The scan now delegates literal skipping to the bounds-safe
helper and keeps `*i <= len` in every arm; `depth` only decrements on `)`.
A Rust unit pins the three reproducer shapes. The expanded fuzz corpus
(**3168 cases across three seeds**, now 1056 per seed) and the 187-check
lifetime gate are clean; Clippy `-D warnings`, rustfmt and the preprocessor
units remain green.

### CI red-team: the strict `#if` check meets the Clang resource headers

The trailing-token rejection added for withdrawn operators (`#if FOO(1)` must
diagnose, GCC 16.2 parity) turned a previously silent gap into a hard error in
Clang 23.1.5's own resource headers: `__stddef_size_t.h`, `__stddef_null.h` and
seven siblings guard their textual fallback with
`(__has_feature(modules) && !__building_module(_Builtin_stddef))`, and
`xmmintrin.h` does the same for `_Builtin_intrinsics`. GCC does not define
`__building_module`, so it rejects those headers too; Clang defines it as a
builtin. LCCC consumes the Clang resource headers on every target, so the
operator is now a first-class Clang-compatible query (same `#undef`/`#define`/
push-pop lifetime as `__has_warning`) that truthfully answers **0** — LCCC has
no Clang modules. The resolver mirrors Clang 23.1.5's measured diagnostics
(too few/many arguments, nested parentheses, module-name grammar, unterminated
invocation, missing-`(` recovery that absorbs exactly one following preprocessing token).

Differential probe: 14 shapes, LCCC vs Clang 23.1.5 — 12 exact matches
including every diagnostic wording; the two remaining differences are the
Clang-only `-Wbuiltin-macro-redefined` directive warnings (LCCC's documented
persona emits none for its Clang-compatible extensions, exactly as for
`__has_warning`). The former macro-operand divergence is fixed: the macro
expander now preserves the RAW operand of active query operators whose
resolver owns the operand-expansion rule (`__building_module`, `__has_warning`,
and the GCC feature/builtin/attribute queries), including operators produced
by object-like aliases and argument prescan. `__building_module(M)` with
`#define M foo bar` answers 0 instead of erroring, `__has_warning(W)` with
`#define W "-Wcpp"` requires a literal exactly as Clang does, and
missing-`(` recovery follows the measured compiler persona instead of leaving
stray tokens (see the next section).
The lifetime gate now compiles the real Clang `stddef.h` through all four
target drivers and pins the raw-operand macro contexts (244 checks with the
oracle present), and the fuzz corpus covers the tenth operator (1088 cases/seed
at count 800).

Two more CI-only defects found and fixed in the same pass:

* The new `Feature capability truthfulness` step was added to `frontend-b`
  without registering it in `.github/ci-lane-plan.json`; the lane-plan audit
  (and the SIMD pack's `test_13`) correctly rejected the hand-added gate.
  Registered with its measured worst-of-3 cost (0.63 s) and provenance.
* This follow-up cited rucc's own docs/UNVERIFIED.md inventory as a backtick path, which
  the doc-link gate resolves against THIS repo; the citation is now prose.

### Host biarch include fallback (found by the local fast mirror)

The rootless i386 sysroot staged by `scripts/ensure_i386_multilib_safe.sh`
ships the 32-bit glibc stubs and crt/libgcc but deliberately not the
arch-independent `bits/*.h`; the wrapper recipe supplies those with
`-idirafter /usr/include/x86_64-linux-gnu`. The i686 driver's
`LCCC_SYSROOT` overlay admitted the sysroot's `gnu/stubs-32.h` marker but
not the host `bits/` tree, so `<stdio.h>` resolved from the host and every
`bits/*` include failed. `apply_i686_target` now admits the host multiarch
directory as a late fallback when the sysroot supplies the 32-bit marker and
the host actually carries `bits/libc-header-start.h`; the sysroot stubs still
win for `gnu/stubs-*`. With the verified Bookworm biarch packages installed
(no live libc upgrade; exact `2.36-9+deb12u14` match), the previously failing
local gates pass: gcc17-builtin-semantics, trapv-boundary-matrix-m32 (910
expected traps), and builtin-floor-float-narrowing.

### Missing-`(` recovery: one token, two compiler personas

A follow-up oracle matrix (`/tmp/mp.c`, `/tmp/mp2.c`, `/tmp/mp3.c`,
`/tmp/mp4.c`; GCC 16.2 and Clang 23.1.5, `-std=gnu23 -E -P`) exposed three
recovery defects in the raw-operand shield:

* Missing-`(` recovery absorbed only identifiers. Both compilers absorb exactly
  one preprocessing token of **any** kind (string, number, character, or
  punctuator) and then emit `missing '(' after '<op>'` with value `0`.
* Clang-persona operators (`__has_warning`, `__building_module`) must shield the
  raw next token: `F(1)` with `#define F(x) 99` yields `0(1)`, not `099`.
  GCC-persona feature/builtin/attribute operators instead expand normally and
  let the resolver absorb one expanded token: `M(1)` with `#define M foo bar`
  yields `0 bar(1)`, and `F(1)` yields `0`.
* A function-like macro whose replacement list ends in an active query operator
  (`#define Q() __building_module`) bypassed the trailing resolver because the
  operator is compiler-owned rather than a user `MacroDef`. `Q() F(1)` therefore
  expanded `F` before recovery. The trailing-resolution predicate now includes
  active raw-operand query operators, and Clang-persona absorption stops the
  rescan so the absorbed token cannot be expanded a second time.

The implementation adds bounded, literal-aware `skip_blanks_and_comments` and
maximal-munch `pp_token_len` helpers (including prefixed literals, digraphs,
`%:%:`, and C23 digit separators), uses them in every missing-`(` resolver and
in the balanced-argument scanners, and keeps the GCC/Clang persona split in all
three macro-expansion paths. Five Rust units pin the tokenizer/recovery
contracts; the lifetime gate grew from 227 to **244 checks** with direct
GCC/Clang parity cases. Fastbuild is warning-free; the frontend gate, 3264
seeded fuzz cases, Clippy `-D warnings`, rustfmt, and **125 preprocessor units**
are green.

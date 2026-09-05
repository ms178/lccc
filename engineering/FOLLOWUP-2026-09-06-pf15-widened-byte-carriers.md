# PF-15 follow-up — byte-carrier narrowing with measured RA/layout guardrails

**Date:** 2026-09-06

**Base:** `a3f5a3174328fa1f8facdf99cfec70b828781870` (latest `ms178/lccc` main merge, PR #421)
**Scope:** the signed-byte condition/comparison carrier form emitted for `strcmp`-style loops; the change is deliberately not a general unsigned cross-block narrowing pass.

## Decision

**Landed:**

1. Replace a *single-use* widening integer value used solely as a branch condition with its narrow source.  `widen(x) == 0` iff `x == 0`, so a `CondBranch` observes exactly the same truth partition.
2. Replace two *single-use*, identical **signed** widening casts feeding a signed/equality comparison with a comparison of their narrow sources.
3. Keep the pre-existing backend adjacent zero-extension fold.  Do **not** make the new SSA-level pair rewrite cross an intervening instruction for unsigned roots yet.

The final policy is intentionally asymmetric.  The signed carrier is an observed hot gap and removes expensive `movsbq` relay values.  The initially broader U8 pair rule was semantically valid but made the real Expat quote scanner reproducibly slower on this target.  A lower static instruction count did not outweigh its register-allocation/code-layout perturbation.

The two independently controllable emergency / measurement gates are retained:

```text
CCC_NO_WIDENED_COND=1
CCC_NO_WIDENED_CMP_PAIR=1
```

They disable only their named PF-15 transform and are used by the A/B harnesses below.

## Correctness and live-range argument

### Semantic proof

For an integer widening conversion, zero is injective: the converted value is zero exactly when the source is zero.  That licenses a condition rewrite, but **not** a truncation, pointer, or floating conversion; the helper rejects all of those.

For matching signed roots, sign extension preserves equality and signed ordering.  `Eq`, `Ne`, `Slt`, `Sle`, `Sgt`, and `Sge` therefore retain their predicate at the narrow type.  An unsigned comparison of sign-extended negative values observes the high extension bits, so it is explicitly rejected.  Mixed source/destination types and same-width/non-widening casts are rejected too.

### Why single use is a real live-range constraint, not a heuristic label

The simplifier counts every use of a cast result, including terminator, opaque, inline-asm, and atomic operands via `for_each_used_value`; it only rewrites a carrier with exactly one use.  The rewrite therefore deletes the old carrier rather than retaining it next to the narrow source.

For source definition `s`, cast definition `c`, and only consumer `u`, the original intervals are `[s,c]` and `[c,u]`; the replacement has `[s,u]`.  Their total length is equal:

```text
(c - s) + (u - c) = u - s
```

If the source already has a later independent use, replacing the carrier can only remove `[c,u]` from the total live-length accounting.  Thus this admission cannot increase aggregate value live-range length.  It does not claim that register allocation or instruction placement is free — the Expat result below is exactly why the unsigned SSA pair form remains refused.

## Programmatic tests

| Gate | Result |
|---|---|
| Simplifier unit tests: signed mapping, unsigned refusal, mixed/non-widening refusal, generic dispatch, and multi-use veto | 6 focused tests pass (`test_cmp_widen_pair*`, `test_widened*`) |
| `tests/regression/widened_byte_chain.c` | exhaustive 256×256 signed-char `strcmp` first-byte/termination cases pass against GCC |
| `tests/regression/widened_signed_byte_relops.c` | exhaustive 256×256 inputs × six signed predicates pass against GCC |
| `check_widened_byte_chain.sh` | sees `testb` + `cmpb`, and no register-source `movsbq` relay |
| `check_widened_signed_byte_relops.sh` | every `Eq/Ne/Lt/Le/Gt/Ge` helper retains the matching signed `setcc` after a `cmpb` |
| whole regression/oracle suite | `PASS=638 FAIL=0 SKIP=7`; IR verifier enabled on compilation; boot tree was unavailable and honestly skipped |
| workload output oracle | default and both-gates-disabled configurations pass `strlen_bench`, gzip CRC-32, zlib-ng Adler-32, Expat XML scan, SQLite varint, and glibc memcmp against GCC |

The dedicated relational regression produces narrow signed operations such as:

```asm
cmpb %sil, %dil
setl %dl
```

for every signed relational predicate, so the negative-byte half is tested rather than assumed.

## Static code quality

`kernel_count.py` at `-O2` shows the independently attributable designed shape:

| configuration | `my_strlen` | `scmp` | all 15 kernels |
|---|---:|---:|---:|
| both gates disabled | 10 | 20 | 277 |
| condition enabled, pair disabled | 9 | 19 | 275 |
| condition disabled, signed pair enabled | 10 | 18 | 275 |
| final default (both enabled) | **9** | **17** | **273** |

The 55-source RA census, final default A versus both gates disabled B, is:

| metric | A: final default | B: both disabled | B − A |
|---|---:|---:|---:|
| instructions | 6,850 | 6,864 | +14 |
| register-register moves | 1,096 | 1,094 | −2 |
| stack references | 623 | 623 | 0 |
| pushes | 198 | 198 | 0 |

The four changed functions are `k04_strlen:my_strlen` (+1 B−A instruction), `sieve:count_primes` (+3), `k13_strcmp:scmp` (+3), and `strlen_bench:main` (+7, −2 rrmov).  The final policy removes instructions without increasing any census stack-reference or push bucket; it does **not** present static count as a runtime proof.

A Compiler Explorer four-way check (`gcc16.2`, current Clang, ICC 2021.10, current ICX) on the relational reproducer confirms the key selection result: LCCC now uses `cmpb` for `rel_eq`, `rel_lt`, and `rel_ge`.  It still takes five instructions for a boolean-return helper versus four for most oracle compilers because `setcc %dl; movzbl %dl,%edx; movl %edx,%eax` is not yet a return-register-aware lowering.  That independent gap is recorded as follow-up work below; it is outside the hot loop body and was not bundled into PF-15.

## Runtime evidence and the unsigned red-team result

All dynamic figures are CPU-0-pinned, interleaved shared-VM screens.  This KVM guest has no usable `perf` executable/PMU, so they are wall-clock screening evidence, not cycle claims.

### Positive designed kernel

`tests/bench/k_strcmp_signed.c` is a separate-translation-unit, late-mismatch signed-char `strcmp` kernel.  The fixed GCC-built driver is identical for both arms; only its LCCC-compiled kernel changes.  The benchmark tool now supports an explicit alternate LCCC environment, pins children with `taskset`, verifies outputs, and balances AB/BA order instead of permanently running one arm first.

```text
python3 scripts/bench_kernels.py --kernels strcmp_signed \
  --lccc-alt-env CCC_NO_WIDENED_CMP_PAIR=1 --gcc '' --clang '' \
  --reps 101 --inner 3000

final default:       43.179 ms minimum
pair disabled only:  53.406 ms minimum
B/A:                  1.237x
```

Thus the signed pair form is 19.2% faster by the primary minimum statistic in this deliberately carrier-dense loop.  `bench_run` is byte-identical between arms; only `signed_strcmp_loop` changes from three sign-extension relays plus a 32-bit compare to a second memory sign-load plus `cmpb`.

### Real workload screens after the restriction

Final default A versus **both** pair/condition gates disabled B:

| workload | repetitions | A min | B min | B/A | result |
|---|---:|---:|---:|---:|---|
| `strlen_bench` | 101 | 185.94 ms | 187.30 ms | 1.007 | no measurable difference under the 1% threshold |
| `expat_xml_scan` | 61 | 40.17 ms | 40.27 ms | 1.003 | no measurable difference |

The six-workload correctness gate above remains clean in both configurations.  In particular, final default and pair-disabled Expat assembly are identical: the eligible pair there has U8 roots and is intentionally left to the adjacent backend path.

### Rejected broader U8 SSA pair rule

Before the signed-root restriction, 101 alternating Expat rounds found default/both-disabled `41.89/40.14 ms` (B/A 0.958); isolating the pair gate for 61 rounds gave `41.84/40.46 ms` (B/A 0.967).  Disabling only the condition transform was neutral (`41.81/41.87 ms`, B/A 1.0015).

The problematic U8 quote scan replaced:

```asm
movzbl %bl, %r14d
cmpl   %r14d, %esi
```

with `cmpb %bl, %sil`, and also changed allocation/frame/layout.  In one controlled assembly experiment, the broad enabled loop target started at byte 63 of a 64-byte line.  Inserting one `nop` immediately before the target moved it to the line boundary and changed the 101-round minima as follows:

| executable | minimum | fastest-third mean |
|---|---:|---:|
| broad U8 pair enabled | 42.731 ms | 43.004 ms |
| same executable + one target-padding NOP | 40.533 ms | 40.961 ms |
| broad pair disabled | 41.001 ms | 41.324 ms |

This establishes a placement sensitivity, not a license to insert ad-hoc NOPs.  It is the reason static `-4` instructions was rejected as a runtime justification and why unsigned cross-block pair narrowing is not enabled by default.

## Tooling repair included with PF-15

`scripts/bench_kernels.py` had claimed interleaving but emitted every arm in the same within-round order.  It now reverses order every round and rotates multi-arm order, supports a named `lccc-alt` environment arm without object-file collisions, records that environment in the report, and pins timed children to an allowed CPU by default (or explicitly reports why pinning is unavailable).  This made the pair-only experiment reproducible without mislabeling an alternate LCCC binary as GCC.

## Follow-up work

1. **Do not re-enable unsigned cross-block pair narrowing from instruction count alone.**  First add an objective loop/block-placement model or profile-guided alignment experiment with code-size limits.  The existing PGO block-alignment mechanism only acts with a profile; broadly aligning all loop labels is not validated.
2. **Return-aware boolean lowering:** teach RA/emit to generate `xor %eax,%eax; setcc %al; ret` when a boolean comparison directly returns.  The oracle shows the precise 5-to-4 instruction gap.  Measure a caller-heavy workload; do not optimize a cold standalone helper merely to win a count.
3. **Measure I16 and adjacent U8 separately before broadening.**  The final policy only rejects the new cross-instruction SSA pair form.  The mature adjacent backend zero-extension fold remains enabled and has a different liveness/layout surface.
4. **Repeat on bare metal with PMU data** before treating the 1.237× carrier-kernel ratio as a machine-independent result.  The VM evidence is sufficient to reject the Expat regression and support the narrow default, not to assign exact microarchitectural causality.

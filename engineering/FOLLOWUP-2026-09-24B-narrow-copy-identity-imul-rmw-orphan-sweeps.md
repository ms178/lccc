# Follow-up S05→S06 — narrow copy identity, imul NDD phantom-RMW, orphan retirement, popcnt baseline

Session date: 2026-09-24 (continuation of the S05 oracle-distillation session).
Base: `07ac7c2` (unchanged upstream tip). A fourth harness wipe hit at
session start (took `~/.cargo`, `~/.rustup`, `.git`, swap); recovered
byte-exact again from `artifacts/lccc-src.tar.gz` + `lccc.bundle`
(regenerated diff SHA-256 == `ms178-1.patch`). Toolchain reinstalled
(rustc 1.98.1), swap re-armed (6 GB `/var/swapfile-lccc`).

## 1. Root causes found and fixed (evidence-first)

### 1.1 `movl` copies were never propagated into consumers (glibc_strstr, fannkuch relays)

`copy_propagation.rs` tracked `movq` copies in `copy_src` and propagated
them, but `movl` copies went into `copy_src32`, which was used ONLY to
shorten `movl` chains — never to retarget consumers. Two hot-loop shapes
paid for it:

- glibc_strstr Horspool scan: `movzbl (%r10,%r9), %eax; movl %eax, %r9d;
  cmpl %r9d, %esi` — a staging relay per iteration.
- fannkuch swap: `movl %ebp, %eax; movl %eax, (%r8,%r9,4)` — a staging
  relay per element swap.

FIX: the LOW-32 IDENTITY contract (N1–N4, documented at the definition):
a `movl` reg-reg copy proves bits 0..=31 of the destination family equal
bits 0..=31 of the source family (the zeroed upper half is outside the
identity). Two new application sites consume `copy_src32`:

- N2 (consumer retarget): a consumer whose every occurrence of the family
  is 32-bit-or-narrower (never the 64-bit name — that reads bits 32..=63;
  never a legacy `%ah/%ch/%dh/%bh`) and that does not WRITE the family at
  any width (the result must land where the program expects; subsumes
  self-RMW) gets its occurrences retargeted. Address operands are
  structurally excluded: base/index spellings are 64-bit names.
- N3 (store-source retarget): `movX %S, MEM` reads exactly width X of the
  family — `movq` requires the full identity (`copy_src`), `movl/movw/movb`
  require only `copy_src32`. Only the source operand is rewritten.

Frame-family policy (was: blanket exclusion of %rsp/%rbp copies):
`parse_reg_to_reg_movl` now admits `%ebp` as a SOURCE (the RA homes values
there — fannkuch keeps `perm[i]` in `%ebp`), because the narrow identity
structurally cannot leak `%rbp` into addresses or frame-manipulation forms.
`%rsp` stays excluded at both ends (implicit-writer world; zero demand),
destinations in families 4/5 stay excluded (frame-setup shapes), and
`parse_reg_to_reg_movq` keeps its full 4/5 exclusion (no measured demand
for 64-bit frame-value propagation; smaller audit surface).

Measured (fastbuild, `-O2` on the benchmark sources): strstr `movl %eax,
%r9d` relays 1 → **0** (hot loop now 5 insns + branch, was 6 + relay);
fannkuch `movl %ebp, %eax` relays 1 → **0** (store folded into
`movl %ebp, (%r8,%r9,4)`). Sieve fold preserved (0 `movsbq`).

### 1.2 3-operand `imul` was modelled as a phantom RMW of its destination

`is_read_modify_write` is the single source of truth for the dest-only
contract. It classified `imull $imm, %src, %dst` (the magic-division
lowering — glibc_strstr's `% 26`, fannkuch's index math) as
read-modify-write, i.e. a PHANTOM READ of the destination's old value.
Isolated liveness probe (`FileLiveness::live_after`) proved it: the relay
in front of `imull $5, %ebx, %r9d` reported live-out (kept forever),
while the `leal` shape reported dead correctly.

This is the same bug class as the rorx drift (S03) — a missing mnemonic
form in the RMW model — one level deeper: OPERAND FORM, not just
mnemonic. FIX: the predicate is now operand-count aware for the
non-destructive forms of otherwise-RMW mnemonics:

- `imul`/ALU `add sub adc sbb and or xor` family: 3+ operands ⇒ dest-only
  (the extra-destination form: legacy 3-operand `imul`, APX NDD);
- `neg not inc dec`: 2+ operands ⇒ dest-only (APX NDD unary form);
- `shld`/`shrd` DELIBERATELY stay RMW at 3 operands (bits shift through
  the destination); 1-operand `imul`/`mul`/`div` stay conservative.

Exact-mnemonic matching (SSE look-alikes `addss`/`imulsd` never enter);
SIB-aware operand counting (`last_top_level_comma`); trailing `#`
comments stripped before counting (a comma inside a comment must not
change the count). The `src == dst` degenerate spelling (`imull $5, %r9d,
%r9d`) still reads the family — the liveness caller's `src_reads_dest`
guard (verified at liveness.rs:962) covers it exactly as it covers
`popcnt %eax, %eax`.

### 1.3 Phase-2/3/4b folds orphan dead pure writes with nothing to retire them

Pipeline defect: `eliminate_dead_pure_writes` (the only FileLiveness-proven
pure-write killer) ran in Phase 1's fixpoint ONLY. Phase 2 runs ONCE after
that (copy propagation, relay folds, slot DSE) and Phases 3/4b loop — every
one of those folds orphans staging copies (`copy_fold`'s own comment
promised "the copies this fold orphans are retired in the same round" and
the only instrument that can prove it never ran there). Observed as the
surviving `movl %ebp, %eax` after N3 retargeted its only consumer.

FIX: `retire_dead_pure_writes` (shared helper — local fixpoint, because
killing one relay exposes the next in a `mov A,B; mov B,C` chain) wired at
four sites, all under the SAME `CCC_PEEPHOLE_SKIP=dead_pure_writes` knob
as Phase 1: end of Phase 2 (feeding Phase 3), inside Phase 3's and 4b's
fixpoint loops, and a final sweep after Phases 4c–8b so the terminal
invariant holds: **no provably-dead GP pure write leaves the peephole.**

### 1.4 POPCNT missing from the x86-64-v3 default baseline

The BMI/ABM sticky-denial fix (S05) covered bmi1/bmi2/lzcnt but left
`popcnt` explicit-only, so default `-O2` emitted the 15-instruction SWAR
bitcount where `-mpopcnt` emitted `popcntl`. POPCNT is an x86-64-v2
member (v3 carries it a fortiori). FIX: `popcnt_effective()` mirrors the
other `*_effective()` contracts; sticky `-mno-popcnt` denial, later
`-mpopcnt` lift (GCC last-explicit-wins), explicit v1 ceiling. Verified:
default emits `popcntl`; `-mno-popcnt` emits the SWAR fallback; new CLI
test pins all four cases.

## 2. Tests added (all green; suite 3329/0, +14 vs S05)

- copy_propagation: narrow consumer retarget (b/w/l), store-source retarget
  (movl/movq + low32-refusal for movq), 64-bit-reader refusal,
  address-use refusal (base/index = 64-bit read), dest-write refusal,
  high-byte mixed-line refusal, SetCC-dest refusal, `%cl` shift-count
  refusal, imul 3-op kill e2e + imul 2-op survival e2e, frame-family
  store retarget e2e + full relay-death e2e.
- liveness drift-alarm table: 3-operand `imul`/NDD dest-only set vs
  2-operand `imul`, 1-operand `imul`, `neg`, `shld`, `addsd` RMW set.
- cli.rs: popcnt baseline default / sticky denial / lift / v1 ceiling.

## 3. Verification performed (no guesswork)

- Isolated liveness probes before/after the imul fix (`imul3`/`add2`/`lea`
  matrix) — documented above.
- Real codegen: strstr + fannkuch + sieve assembly dumps compared line by
  line (relays 1→0 both kernels; hot loops shown in the session log).
- Gates: `check_vectorize_isa_gate.sh` PASS, `check_findbit_inline.sh`
  PASS, `check_ch_maj_codegen.sh` PASS (A/B/C), golden codegen gate
  (`ci-codegen-gate.py --lccc`) "all golden workloads within tolerance".
- `cargo fmt` clean; `cargo clippy --all-targets` zero warnings;
  `cargo test --lib` 3329 passed / 0 failed / 7 ignored;
  `scripts/ci_local.sh --fast` green (S06 snapshot condition).

## 4. Remaining TODO (unchanged in priority order)

1. chacha20 2.19× (EPYC): ARX vectorizer cost model is instruction-count
   only; Zen5 vector-int ALU 3c vs scalar 1c. Multi-session scalar RA
   rework (live_range.rs auto-arming + MAX_SPAN_REMCOST).
2. fannkuch swap-loop shaping: `movslq %r9d, %r13; shlq $2, %r13` pair
   (r13 read elsewhere — verify before deleting), IV advance stack spill
   (`leal 1(%rsi), %eax; movslq; movq 16(%rsp); movq %rsi`) — middle-end
   pointer-walk IV would kill the whole `movl %edi, %r8d` SIB-staging
   class (also the remaining strstr staging movl).
3. strstr shift-table fill (256-iter scalar) needs a runtime-splat
   memset/vector-fill variant in `loop_memset.rs` (GCC does SSE
   broadcast); current refusal (runtime fill value) is correct.
4. GCC sieve `cmpb $1; sbbl $-1` idiom needs middle-end VRP.
5. expat 1.31×, mandelbrot ~1.09, sha256 1.32, linux_find_bit 1.42
   (worst case after the inliner fix), sieve 1.25 (fold landed — re-measure
   on EPYC).
6. Cosmetic: `xorl %esi, %esi` before `popcntl %edi, %esi` is a redundant
   zeroing (zero-cost via xor-elim, but a folding target).

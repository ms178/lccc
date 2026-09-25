# S11 row-16 red-team audit (self), 2026-09-24

Scope: every change layered onto `b95b0aa9` for row 16 — the composed exact
last-write law (H1 fix), the abstain law, the window allocator's save-aware
laws, the Origin-tagged allocation pipeline, and this session's triage.

## Findings against my own work

### F1 (critical, fixed): Origin tags desynchronized from inserted reloads
`allocate_window` built the `Origin` tag vector by inserting at ascending
indices while the instruction vector inserted the same traffic in REVERSE.
With ≥2 reloads at distinct indices the vectors misalign; a CORE relay copy
(`slot → scratch → phi home`) inherited a foreign vreg's inserted-traffic
tag, was classified `ScratchOrClobber`, and the flush overlay recorded a
Clobber on a register whose resident value was still live — refusing the
latch's fused compare in `net/core/rtnetlink.c do_setlink` (value 3175).
Baseline `d2db7cd2` compiles the file; the refusal was a genuine S11
regression. Fixed by mirroring the reverse insertion; regression unit test
`origin_tags_stay_aligned_with_reverse_inserted_reloads` locks the exact
two-reload shape. Verification: rtnetlink.o + all seven prior ICE repro
targets assemble clean; `do_setlink` .text size equals baseline (other
functions within the object differ ≤ 8 bytes each; total .text size
identical at 71188).

### F2 (audit honesty, fixed): two dead "group-lineage" patches removed
During triage I first added two "sharing-group lineage" extensions
(`note_staging_target`, overlay SC arm) under the belief that a value could
share a group slot while being homed elsewhere. That belief is FALSE:
`home_sharers[R]` is built directly from `reg_assignments`, so membership
implies `ra[v]==R` and the exact-home test already covers it. Both patches
were unreachable dead code (proven by the tracer: the misclassified event
carried `writer=834` which is NOT in r4's group — the extension tested
exactly that and passed through). Removed; the origin fix is the complete
correction. Lesson recorded: every new law must be shown to FIRE on its
justifying trace before it is kept.

### F3 (fixed, pre-existing surface): `vptest` lost the memory form
The integrated assembler only encoded `vptest reg, reg`; the kernel's
`aes-gcm-aesni-x86_64.S` `_test_mem` macro uses `vptest mem, reg` and a
fresh tree assembles that file with lccc as `AS`. Added the VEX memory arm
(`C4 E2 79 17 21` for `vptest (%rcx),%xmm4` — byte-verified against the ISA
spec: VEX.R/X/B=111, mmmmm=0F38, L=0, pp=66, modrm reg=xmm4 rm=(%rcx)).

### F4 (fixed): refused-arm overlay under-modeling
When `allocate_window` refused but the no-skip re-resolution fully resolved,
the emitted sequence carried an EMPTY write map, so the overlay skipped the
window and stale write-state facts leaked across it. Now classified with
`window_last_writes_core` (the same law the fast path uses).

### F5 (hygiene, fixed): `[WS-m11]` tracer hardcoded phys 11
Generalized to `[WS-m]` driven by `LCCC_DEBUG_WS_REG` like every other
tracer in the inventory.

## Re-audit of the retained laws (adversarial pass)

* Join law (block entry): any-Clobber poisons; Def agreement over the group
  keeps a Def; any absence abstains. Sound under the group-lineage invariant
  (two values sharing a home with overlapping liveness are chain-same-value,
  so a member's definition write carries the group's content). Verified the
  `views[0]` shortcut is sound: differing member Defs per path imply the
  contents are chain-equal for any live consumer.
* Abstain law: no recorded edge ⇒ state cleared ⇒ Layer-2 abstains and the
  calibrated layer answers as pre-dataflow. This is the conservative
  semantics the H1 adjudication demanded; hugetlb `demote_store` proves it
  fires (EXIT=2 before, clean after).
* Staging/call/raw classifiers unchanged this session; unit suite 3285/0
  (7 ignored), clippy 0 across all targets, rustfmt clean.
* Diagnostics kept under the strip-or-justify law, justified here: [WS-ev],
  [WS-win], [WS-m], [WS-busy], [NOHOME], [NOHOME-sib], [alias-refine],
  CCC_MI_STREAM — all env-gated, all fire-only-on-debug-paths, and each was
  load-bearing in at least one S11 triage (the [WS-*] family found F1;
  [NOHOME-sib] proved the freshness blind-spot shape).

## Open items (next rows)

1. sha256_transform (−78 vs gcc) and base64_enc (−41) are vectorization /
   vectorized-LUT gaps, not RA gaps — row 17+ candidates after vec_arx.
2. `icc` on chacha20_block is degenerate (2331 insns) — noted, no action.
3. Allocation-refused replay path still re-runs `resolve_stack_vregs`
   twice on the refusal path only; acceptable, revisit if a profile shows it.
4. AVX-512 module assembly: `aes-gcm-vaes-avx512.S` needs
   `vshufi64x2 $0, %zmmH, %zmmH, %zmmH` (EVEX.NDS.512.66.0F3A.W1 43 /r ib
   with high-zmm R'/B' register bits). This requires the zmm register table
   + EVEX prefix infrastructure as one coherent unit — tracked for a
   dedicated assembler row. The validated kernel workload gate is
   `build_kernel_full.sh` → target `vmlinux` (the configuration that
   produced the 16/16 QEMU boots); the AVX-512 crypto files are loadable
   modules outside that gate. The `vptest` memory-form fix from F3 stays:
   it is a genuine completeness fix, byte-verified, and inside the first
   module any AVX user would hit.

## Session-2 additions (fallthrough law, ANDN hardening, i686 parity)

### F7 (critical, fixed): CondBranch fallthrough home absorption — pci_irq ICE
After a conditional branch, `home_fallthrough` was set unconditionally, so an
UNRELATED successor's state join absorbed the pre-branch home state (the
emitted jcc+jmp has no architectural fallthrough into a sibling). The join
attributed homes from a dead sibling's lineage; the first consumer that
trusted a stale home in `drivers/pci/pci_irq.c` hit the operand_to_rax
v=312 home-stale ICE. Architectural fix in generation.rs:
`set_home_fallthrough(state_ref().next_block_label == Some(true|false))` —
fallthrough exists only into an actual branch target. Also removes the
latent miscompile class where baseline blessed such reads via a dead
sibling's stale mark (the audit-H1 refutation shape). All nine kernel ICE
repro targets assemble clean; lib suite 3367/0. Commit `2f21fd92`.

### F8 (critical, fixed): emit_and_not_impl read homes without freshness (B4)
The BMI andn emission read the non-negated operand's home register
UNCONDITIONALLY — a clobbered home (staging, in-place compute, MachInst
window) fed a garbage mask to every Not+And consumer under BMI. This is the
same class the audit called free_area_init_node; it predates S11. Fixed by
routing the read through fresh_home_of (composed mark/dataflow law) and
folding to the BMI memory form (andn{q,l} slot(%rbp)) when the operand has a
sound direct slot image instead (slot_fits_width holds, allocas excluded —
resolve_slot_addr returns Direct only for SSA-stable stack slots). Commit
`4375edd8`. check_andn_fusion.sh gained the red-team shape (noinline mask,
intervening clobbering call, all-path compare against the scalar reference
plus an andn-presence pin). Oracle at -O3 -march=x86-64-v3: clear_bits lccc
17 insns = best of gcc 16.2 (4.59x), clang 23.1 (7.00x), icx latest (3.18x);
find_next remains 1.33x-of-gcc (loop-shape gap, row 17).

### F9 (parity gap, fixed): i686 integer-ISA capabilities frozen out
The #607 Target::X86_64 capability lock was correct-but-total: i686 never
received tzcnt/popcnt although the i686 backend fully serves both (ctz/clz
emitters with stop-bit sequences for both hardware classes, popcount
consumer, 'tzcntl'/'popcntl' in the integrated assembler). F3-prefixed
encodings are universal (pre-ABM CPUs decode tzcnt as plain BSF, identical
for nonzero inputs), so the grant only selects between two correct
lowerings. Grant shape: i686 explicit-flag-only (no implicit v3 default —
the i686 baseline promises pre-ABM/pre-SSE4.2 compatibility); -mno-* veto
and the __LZCNT__/__POPCNT__ macro mirror hold; movbe/bmi1 stay x86-64-only
(no i686 emitters — granting without a serving emitter reopens the deferral
cliff). Commit `a3a8c964` with tests/regression/
check_i686_integer_isa_parity.sh (forms/baseline/veto/macro/x86-64-unchanged).

### Verification ledger for the three commits
lib 3367/0; clippy --all-targets 0; fmt clean; alias_fuzz_m32 80/0;
differential_fuzz 80/80; differential corpus vs 105b3176-derived base:
output-neutral (byte-identical asm, identical failure set);
check_andn_fusion.sh + check_i686_integer_isa_parity.sh green;
/tmp/andn_test.c red-team (O3/O2/Os) + gcc-14 agree. Pending at doc time:
v9 vmlinux + QEMU 16/16 (launched), ci_local --fast.

## Session-3 additions (rebase onto #612, upstream audit, arp_create)

### Red-team verdict on upstream PR #612 (86f065a5): AGREE, no defects found
* `ir/provenance.rs` — leaf-typing (only Ptr-typed ParamRef/Alloca/GlobalAddr
  name objects) matches LLVM's inttoptr-may-alias-anything model; Copy
  transparency is forced by the erased int↔ptr lowering; `produces_pointer`
  deliberately NOT looking through Copy is the only choice that keeps
  `p + (int)q` valid.  Fail-closed on loads/arithmetic/phis.
* `bit_idioms::andn_fusion_predicted_at` completing its own And-consumer
  validation is redundant inside Pattern B (and_strict already guarantees
  it) — zero production delta, contract tightening only.  Correct.
* `loop_unroll` fail-closed persist gate: multi-latch / latch-shape /
  no-iv / no-exit / multi-entry-init / dynamic-bound all refuse; the
  depth-8 `depends_only_on_const_and_iv` budget refuses on exhaustion
  (wrong direction would have been an allow).  The value-based exit plus
  the depends-only check is the right soundness/cascade trade.  The
  CCC_UNROLL_LEGACY_PERSIST_GATE A/B knob keeps the refusal honest.
* laundered int-param copies taking memmove: sound (memcpy+overlap is UB;
  without a proven root memmove is the only correct choice).  The perf
  ceiling (small-constant-length inline expansion) is a follow-up, not a
  defect.  `check_provenance_int_param.sh` PASS=3 locally.
* CI gate hardening (missing-GCC fails the differential) exposed no env
  gaps here (gcc-14 present; both my gates are GCC-free by design).

### F10 (critical, fixed): def matcher omitted Copy — arp_create ICE
Kernel `net/ipv4/arp.c arp_create` (value 1408: Copy-defined, home
clobbered by six-argument call staging, no slot, no acc) iced in
operand_to_rax: `get_defining_instruction` had never reported Copy defs,
so `rematerialize_stale_into_rax`'s Copy branch — sound by construction
(Copy is the identity; the source is re-derived with the ordinary
freshness/slot discipline at the consumption point) — was dead code, as
were the explicit Copy arms of calls.rs's const-chain and
global-addr-chain resolvers.  Fixed by reporting Copy dests (commit
`4c0e200e`); the walk is factored into `find_defining_instruction` with
a fails-first unit test.  arp.o now compiles clean; all six consumers
audited: three dead Copy-followers resurrected, GlobalAddr-only
classifiers unchanged.  This closes the last open S11 kernel-ICE item.

### Work-order addition (diagnosed this session, staged): local const-
### aggregate static promotion
`i686_alu_chains` main is 193 insns / 75 spills vs clang 115 (rank2 gap
78) because the local `const struct kernel ks[]` (all-constant init:
string addresses, function addresses, ints) is runtime-constructed on a
316-byte stack frame (51 leaq+store pairs + memset) while GCC/Clang emit
it as .data.rel.ro with link-time relocations — zero runtime code, and
the spill storm around the driver loop disappears with it.  Promotion is
sound for const-qualified locals with globally-evaluable aggregate
initializers (C11 6.7.3p7: modification of a const-defined object is
UB — GCC/Clang rely on the same license; writes then fault in .rodata
exactly like theirs).  Design: at the declarator site, const+aggregate+
Initializer::List → build the global bytes through the existing
global_init_compound machinery and bind the name to a GlobalAddr instead
of an Alloca; _Alignas must be carried; volatile members reject; runtime
initializers fall back to the alloca path.  Benefits all targets
(x86-64/i686/ARM) and every kernel id/name/ops table.  Staged as the
lead item for the next session (frontend plumbing across VarSlot
binding is a focused change that deserves its own red-team cycle, not a
tail-end rush).

### Verification ledger, session 3
Rebase onto 3275b8b1 (PR #612) clean; lib 3388/0 (upstream +20, +1 own);
clippy 0; fmt clean; alias_fuzz_m32 60/0; m32_differential 50/0;
differential_fuzz 80/0; alu_torture_m32 MATCH O2+Os; check_andn_fusion +
check_i686_integer_isa_parity + provenance gates green;
check_ci_gate_parity 60 commands PASS; rank2 (-O3 -march=x86-64-v3,
gcc 16.2/clang 23.1/icx latest): sieve count_primes now 1.00x BEST
(57 vs gcc 216) — the old B1 item is resolved; worst remaining gaps are
vectorization-class (zlib_ng_adler32 221, loop_patterns 92, nbody 90)
plus the i686 driver-table class above.  Kernel v10 incremental in
flight at doc time (arp.o fixed; full-sweep regeneration planned).

## Session-4 additions (fortify-string link blocker: root cause + fixes)

### F11 (critical, fixed): __diagnose_as / __diagnose_as_builtin__ folded away
The v10 vmlinux link failed on undefined `__fortify_strlen` (kernfs/symlink.o,
vsprintf.o, dm-sysfs.o) and `fortify_memset_chk` (integrity_audit.o).  Root
cause chain, each link verified with the preprocessed TU:
1. compiler_attributes.h maps `__diagnose_as(builtin...)` to
   `__attribute__((__diagnose_as_builtin__(builtin)))` under
   `__has_attribute(__diagnose_as_builtin__)`; lccc's `__has_attribute`
   allowlist lacked every spelling, so the kernel preprocessed the attribute
   away entirely (verified: zero `diagnose` tokens in integrity_audit.i).
2. The parser had no arm for the attribute; added
   (parse_diagnose_as_attr: records the builtin, skips the arg-position
   list), plumbed through DeclAttributes -> FunctionAttributes (definitions
   carry it too — the fortify family defines and declares in one shape) and
   collected into `diagnose_as_builtins` for BOTH the declaration and
   definition loops.
3. Call lowering folds diagnosed calls to the named builtin (GCC's
   declarative-alias semantics: only __builtin_constant_p-guarded
   COMPILE-TIME diagnostics are dropped, never a defined program's runtime
   semantics).  The kernel's dispatch shape — the non-constant arm of a
   `__builtin_choose_expr(__is_constexpr(...))` discriminator — folds too.
Result: symlink.o / vsprintf.o / dm-sysfs.o all clean (0 fortify undefs).
tests/regression/check_fortify_diagnose_as.sh pins the extern-decl shape,
the budget-exhaustion shape, and the kernel choose_expr+definition shape.

### F12 (open, precisely scoped): inliner round starvation — fortify_memset_chk
integrity_audit_message retains ONE call to `fortify_memset_chk(size=-1,
p_size=-1, p_size_field=-1)` (a memset macro expansion whose object size is
unknown).  The callee is `extern inline __gnu_inline__ __always_inline__`
with a body that IS in the module (build_callee_map lists it: 65 blocks /
40 insts, direct_calls=1), yet no selection round ever picked it: the
fixpoint scheduler re-visits sites inside fresh clones (sized_strscpy
inlined 190x into integrity_audit_message in one recorded run) and the
fortify site never reaches a selector pass before the body is dropped by
the inlined-static cleanup — leaving a call with no definition.  An
unconditional budget bypass was tried and REVERTED: it removes the starvation
but the same churn makes dm-sysfs.o compile for >500 s.  The GCC-faithful
fix is scheduler-level (dedicated always_inline sweep before the ordinary
fixpoint, or clone-scoped site dedup), staged as the next session's lead
item together with the const-aggregate promotion (see Session-3).
vmlinux/QEMU remains blocked on exactly this one TU.

### Verification ledger, session 4
lib 3388/0; clippy 0; fmt clean; fortify gate PASS (three shapes);
alias_fuzz_m32 60/0; m32_differential 50/0; differential_fuzz 80/0;
alu_torture MATCH; check_andn_fusion / check_i686_integer_isa_parity /
provenance gates green; arp.o clean (Copy-arm fix); snapshot APPLIES-CLEAN
on 3275b8b1.

## Session-5 additions (rebase onto #616; F12 FIXED — Phase 0 drain)

### F12 (was open, now FIXED): Phase 0 mandatory always_inline drain
The scheduler starvation behind integrity_audit.o's surviving
`fortify_memset_chk` call is fixed architecturally.  The ordinary fixpoint
inlines ONE site per round under a 200-round cap, and always_inline callees
that replicate through macro dispatchers (each inlined clone re-entering
the same dispatcher — net site growth per inline) burn rounds faster than
the queue drains; the contract site starved.  The fix adds a Phase 0
BEFORE the ordinary economics, with three laws:
  * priority — sites present at phase entry (block label <= the module-wide
    label maximum snapshot; clones always take labels strictly above it)
    drain before any clone-born site;
  * termination — each entry site is consumed by its own inlining, and
    clone-born sites share one bounded work counter (2x entry sites + 16),
    so a recursive dispatcher chain cannot chase the phase forever;
  * no budget gate — a mandatory inline is not an allocation decision, and
    phase successes do not touch the economics budgets.
The earlier crude fix (raising/removing the always_inline allowance inside
the economics loop) was tried and REVERTED: it removed the starvation but
let the same replication churn run unbounded (>500 s for dm-sysfs.o).
Phase 0 bounds the churn structurally: integrity_audit.o now compiles in
15.4 s with ZERO fortify undefineds; symlink.o/vsprintf.o/dm-sysfs.o in
22.5 s combined — all four previously link-breaking TUs clean.
Regression lock: `inline_run` unit test
`phase0_drains_contract_sites_despite_replicating_dispatchers` — verified
FAILS-FIRST (with Phase 0 disabled the contract site survives; the assert
names the law).  An unconditional-budget fix was rejected in review
because it trades a correctness bug for a compile-time cliff; the
label-snapshot priority + bounded work counter is the shape GCC's
cgraph always_inline bootstrapping uses (mandatory edges first, no cost
model).

### Rebase note
Rebased onto 5624367b (PRs #613-#616: GVN cross-block Mul CSE, unroll
verdict pins, i686 assembler HLE/EVEX/GFNI).  One conflict
(encoder/mod.rs vptest mem-form) was the same fix landed both sides —
kept upstream's body, merged my comment payload.

## Session 6 — driver honesty laws, depfile architecture, never-null CVP law

Base: 9667a59f (PR #617) + 5 commits. Every claim below was verified with a
running artifact (gate, unit test, oracle manifest, or live kernel log), not
asserted.

### 1. Case-insensitive branch mnemonics (3380dbad)
GAS mnemonics are case-insensitive; `mnemonic_takes_label` matched as written,
so the kernel's uppercase `CALL`/`JMP`/`JE`/`LOOP` (ftrace_64.S macros) lost
bare-label classification and died as "unsupported call operand".  Fix
normalizes after the ".s" hint-strip.  Red-team: the normalization is scoped
to label classification, not the whole mnemonic table — encoders keep their
own matching.  Pin byte-compares uppercase vs lowercase text sections.

### 2. Debug-info honesty law (618c6494)
lccc emits line tables only (.file/.loc → .debug_line); there is NO
.debug_info DIE emission, and optimization erases spans, so `-g` TUs could
carry zero debug sections SILENTLY.  The 6.18 kernel build surfaced this as
`pahole: .tmp_vmlinux1: Invalid argument` → BTFIDS "failed to find .BTF" —
three link stages away from the cause.  Law: warn per-TU when -g produced no
.file/.loc; GCC-exact -g family (-g0/-ggdb0 disable, -gno-* never enable,
-gsplit-dwarf/-gz degrade loudly, -gdwarf-N/-gN enable).  Red-team
disagreement with the OLD driver: `starts_with("-g")` inverted GCC's -g0.
Full .debug_info emission (kernel BTF) remains the next debug workstream;
until then DEBUG_INFO_BTF=n is the honest kernel config and the warning
fired 9x in the live v11 build to say so.

### 3. Real make dependencies (acc0a4cb) — the kernel-link root cause
The dep writer emitted `target: source` only.  fixdep greps .d files for
CONFIG_* tokens borne by HEADERS; the DEBUG_INFO_BTF y→n flip therefore
never rebuilt fork.o/build_utility.o/xfrm_policy.o (their headers had
swallowed SCHED_CLASS_EXT code when BTF was still y) and vmlinux died with
`undefined __scx_enabled/ext_sched_class/register_xfrm_state_bpf/...`.
Fixed architecturally: the preprocessor records every file it OPENS
(#include, #include_next with per-search-list verdicts, force-includes;
NOT __has_include probes) with a system-directory verdict taken from the
SEARCH STEP THAT MATCHED — GCC's -MMD filter is directory-based, not
bracket-based (red-team check: `#include "/usr/include/x.h"` quoted form is
still a system-dir dep and must be filtered under -MMD).  -M/-MM upgraded
from the `target: source` stub to real dependency-only preprocessing; -MP
phonies; -MT/-MQ honored.  Dedup: lccc by resolved path, GCC by raw string
(GCC lists one file twice via two paths); the unique file set — what
make/fixdep consume — is identical, pinned against GCC in the gate.

### 4. Never-null-address CVP law (41106bf8)
`Cmp{GlobalAddr/Alloca/DynAlloca/LabelAddr/StackSave, NULL}` folds
(C11 6.5.3.2: objects have distinct non-null addresses; frame addresses
derive from %rsp).  lccc emitted `leaq sym(%rip),%rdi; testq %rdi,%rdi;
je` + `sete`+slot-store on every guarded copy into a static buffer.
Scope red-team: casts excluded (inttoptr can re-enter 0), GEPs excluded
(would need UB reasoning on null bases), GetStaticChain excluded (zero
without a static chain), GlobalAddr-vs-GlobalAddr excluded (weak aliases
can legitimately compare equal — symbol identity is NOT address identity
under aliases).  Signed pointer predicates left unfolded by choice.
Hard data: zlib_ng_adler32 `main` 350→337 insns vs icc (gap 277→264),
all four address-test chains gone; oracle artifacts in /tmp/adler2 (transient)
and the rank survey (/tmp/rank_fresh).  Soundness counter-case pinned:
a runtime pointer's null test does not fold.

### 5. Upstream #617 review (TU-closed global DSE, copysign punning, CVP
select folding, XMM-homed FP selects)
AGREE: TU-closed DSE is the right closure — cross-TU escape via already-
emitted globals cannot be unsound if the TU boundary is the emit boundary,
and it demonstrably does not regress the kernel TUs (v11 compiles 0 internal
errors).  CVP select folding composes cleanly with the never-null law
(select conditions are decided through the same decide_bool path — the two
laws share one fact stack, verified by the suite).  XMM-homed FP selects:
agree with homing over spilling (kernel -mno-sse TUs never take this path;
the pipeline's first_sse_reference guard stays the last line of defense).
DISAGREE (minor, recorded): #617's CVP `defs` map was Cmp-only while its own
comment says "value → definition maps" — 41106bf8 widens it; upstream
consumers are if-let-guarded so this is a pure widening, but it should be
folded upstream rather than carried as a delta.

### 6. Residual gaps on the fresh rank (hard data, /tmp/rank_fresh.json)
zlib_ng_adler32 337 vs icc 73 — icc outlines the driver's LCG init and the
remaining lccc delta is const-vector frame materialization (.LCVEC_0 copied
to 544(%rsp) then reloaded): this is the const-aggregate promotion queue
item in miniature (parked with evidence).  nbody 308 vs gcc16.2 118 (FP
vectorization), moving_stats 222 vs 131, i686_alu_chains 193 vs icc 109
(const-table driver, queue lead), loop_patterns 260 vs 179.  No new
structural claim is made for these without the promotion work landed.

### 7. Kernel v11 final state (rebuilt post-rebase, BTF off)
vmlinux: BUILT AND LINKED (63.3 MB, ~2400 TUs, 0 internal errors, lccc CC +
lccc-ld).  bzImage: BUILT AND LINKED (16.3 MB, zstd-22 payload) after three
boot-path fixes (c2b3770c: i686 vmmcall/vmcall, rdrand/rdseed, `.word . -
sym - 1` dot-diff — each GAS-byte-verified, gate check_i686_boot_asm).
QEMU VERDICT: FAIL — early 16-bit real-mode setup triple-faults before
"Decompressing Linux" (qemu -d int: 431 serviced INT8 then an exception
storm; -no-reboot exit 0).  The 16-bit boot codegen (arch/x86/boot/*.c
under .code16gcc + header.S handoff) is the next workstream; the evidence
chain (bzImage artifact, qemu logs pattern, per-TU mem_encrypt repro)
is recorded here so the next session starts at the fault, not at the build.
Note for that session: `file vmlinux` reports "bad note name size" —
lccc-ld note emission fidelity is suspect and is a candidate contributor;
audit Elf64_Nhdr emission first.

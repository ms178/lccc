# PR #711 Review-AI audit — adjudication and follow-up (2026-10-01)

The Review AI audited the PR #711 series (26 commits on `8db75621`, tree
`a3434bec` = this series before the round) and raised six findings. Every
finding was re-derived against the code and re-probed empirically before
any action; none was taken on faith. Verdicts: **F1 accepted (P0,
confirmed on hardware-decodable bytes), F2 accepted (P1, both halves),
F3 accepted (P2), F4 accepted (P2, both halves), F5 accepted (P2),
F6 partially accepted (docstrings yes; duplicate binding yes; recursion
claim disproven; evidence diet rejected)**. One sub-claim inside F1 was
adjudicated AGAINST the audit (the segment-override elision), with the
GAS-probed bytes as the tie-breaker.

| # | Finding | Verdict | Evidence |
|---|---------|---------|----------|
| F1 | `fold_index_into_base` miscompiles 102 VEX/XOP/APX-EVEX memory sites (`%r8`–`%r31` index-only scale-1) | **ACCEPTED (P0)** | reproduced pre-fix: `vmovups (,%r10,1),%xmm0` → `c4 a1 78 10 02` = `vmovups (%rdx),%xmm0`; `andnq (,%r9,1),%rax,%rcx` → `(%rcx)`; APX NDD r10/r16 → `(%rax)`; `vpcmov` → `(%rdx)`. Fixed by the shared folded view (`mem_vex_xb_bits` + 5 APX helpers); 17 new byte-pins objdump-verified |
| F1b | segment elision must re-decide on the folded view (`%ds:(,%rbp,1)` "drops %ds") | **REJECTED** | GAS decides elision on the SOURCE operand (no base → %ds default: explicit ds dropped, ss kept — probed both directions on GAS). The pre-fold scan already matches GAS byte-for-byte; re-deciding post-fold would emit the exact OPPOSITE of GAS's prefix bytes. In 64-bit flat mode both segments are base-0 anyway (byte-parity issue at most, not wrong-code). Pinned by test |
| F2 | `DECLINED-DATA16` fail-open on 32-bit loop rows + wrong SDM counter-width claim | **ACCEPTED (P1)** | the `same is False` bypass rubber-stamped arbitrary bytes as a policy decline; removed — declines are verified-only now (negative test: `90 90 90` → WRONG-BYTES). The counter claim was wrong: CX/ECX is ADDRESS-SIZE selected (objdump renders `67 e2` as `loopw`, `66 e2` as `data16 loop` in both modes); 66 on E0–E3 in 32-bit mode only truncates IP — the jmpw argument — so `data16 loop` unifies in both modes. Mocked suite 41→42 green; live corpus partition unchanged (ok=10 DECLINED=7 both-reject=1), now honestly verified |
| F3 | `i386_exec.sh` memoization 100% ineffective (subshell) | **ACCEPTED (P2)** | `$(...)` forks discard every `_I386_CAP_MEMO` write; restructured to `_i386_ensure_memo` in the parent shell + the two `$()`-capture call sites. Live: 3 calls → 30 ms, exactly one memo entry |
| F4 | `_TEST` splits SIB commas; `_sext_imm32_form` matches a non-existent 6-byte `REX.W+B8` form | **ACCEPTED (P2)** | narrowed to `%reg,%reg` (memory forms have ONE encoding shape — nothing to commute; the old split tore `0x10(%rbx` off `0x10(%rbx,%rcx,4)`); the phantom B8 arm (REX.W B8 is always 10-byte movabs) replaced by the REAL REX2 `d5 18 c7 /0 imm32` sign-extending form (probed: both lccc and GAS emit it for plain values) + the missing `C7 /0` reg-field check. No EVEX arm: MOV is not EVEX-promotable |
| F5 | bare `gcc` at 9 sites; 4 gates lack SKIP-RUN/3-state PASS | **ACCEPTED (P2)** | all 9 sites → `"$GCC"`; comdat/eh_frame/nocfi/vec_dead now emit SKIP-RUN markers and PASS lines that claim exactly what ran; all six gates re-run end-to-end on this no-multilib host |
| F6a | stale docstrings (disp8-range claim) | **ACCEPTED** | fixed with T1 (the fold takes ANY integer displacement since the ICC-parity extension) |
| F6b | duplicate `disp_at` binding in `patch_short_jumps` | **ACCEPTED** | deduplicated (outer binding reused) |
| F6c | recursive `resolve_numeric_name` can emit `.Lnum_1_0+4+8` | **REJECTED** | disproven: `parse_integer_expr` folds the whole offset at the FIRST separator (`1f+4+8` → base `1f`, offset 12), and the returned base is symbol-charset so it can never split again — the recursion is depth-1 by construction. Probed: `jmp 1f+4+8` assembles byte-identical to GAS. Clarifying comment added |
| F6d | delete the 20 godbolt dump files + 6,575-line rank.json | **REJECTED** | this is the S10 evidence convention (297 → 9 files, already a 55k-line trim): the top-3 .s dirs are the working material for the #1 TODO (nbody 188 / moving_stats 90 / i686_alu_chains 86 codegen round), and rank.json's `detail` (per-loop insns/loads/stores/spills/branch/vector counts, 72 KB) is the machine-readable backing of rank.md's table — re-deriving either means re-running a rate-limited remote survey at the start of exactly the session that needs them |

## The F1 fix in one paragraph

The fold decision is now made ONCE and shared by every prefix family that
pairs with `encode_modrm_mem`: `emit_rex_rm` and the five APX-EVEX memory
helpers extract their `((B,B4),(X,X4))` bits through the single
`folded_addr_ext_bits` fold+extract helper (the follow-up hardening: the
invariant is structural — a future prefix emitter has one obvious function
to call, and re-extracting from the raw operand is no longer the
copy-paste default), and all 63 VEX/XOP call sites derive `(X, B)` through
the new `mem_vex_xb_bits` — so the extension bit moves X→X4 into B→B4 in
lockstep with the register across REX, REX2, VEX, XOP and APX-EVEX.
AVX-512 `encode_evex_mem` intentionally stays unfolded (no corpus row
evidences the EVEX form yet; it encodes the raw operand, so its
`evex_addr_bits` prefix stays consistent with its own ModR/M) — pinned by
a test so the asymmetry is a documented decision, not an accident.

## What the audit got right that earlier rounds missed

The S09 session's own claim "every GPR class folds incl. EGPR/REX2" was
true for the REX/REX2 paths and silently false for VEX/XOP/APX-EVEX — the
fold corpus (156 rows) was GPR-only, so neither asm-diff (byte-equality
vs GAS) nor encdiff (which never had those rows) could catch it. The
audit's contribution was reading the call graph instead of the tests:
`encode_modrm_mem` has 204 callers, and only `emit_rex_rm`-paired ones
folded. That is a real methodological lesson and it is recorded here so
the next encoder change audits the FULL caller set of any shared emitter,
not the corpus that happens to exist.

## Validation this round (fast gates only, per policy)

Build fastbuild `-j2` clean; `cargo fmt` clean; encdiff mocked suite
42/42; the six touched regression gates green end-to-end on this host
(x86-64 legs fully asserted, -m32 legs loud SKIP per taxonomy);
`index-fold-64.insn` whole corpus BEATS=148 ok=8 zero bad, −465 B vs GAS
(the S09 headline, reproduced); `data16-branches-32.insn` partition
unchanged. The full `cargo test --lib` + clippy + `ci_local.sh --fast`
battery ran at the end of the round (see the worklog); slow gates are
hosted-CI-only by policy.

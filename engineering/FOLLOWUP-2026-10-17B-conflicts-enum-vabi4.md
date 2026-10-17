# Follow-up 2026-10-17B: PR #852 conflict resolution, enum semantics, AUD-VABI-4, red-team checklist

Session 3B. PR #852 (head `8c204deb`) was reported **CONFLICTING/DIRTY** by
the #854 red-team against main `f874d1d` (which absorbed #851 audit refresh
and #854's sysroot-boundary fix). This session resolves every landing
blocker, re-does the enum-semantics work lost to harness wipe #4, and adopts
#853's AUD-VABI-4 one-lane FP vector fix after empirical confirmation.

## Merge resolution (6584ffd1b: 8c204deb ⊕ f874d1d)

Seven conflicted files, resolved by three-way semantics, never wholesale:

| File | Resolution | Rationale |
|---|---|---|
| `src/backend/linker_common/resolve_lib.rs` | **main (#854)** | Boundary-checked `strip_sysroot_prefix` + deterministic temp-path test strictly supersede my naive prefix strip (their GitHub run 39046286866 exposed my host-dependent test as red; my `/opt/root` vs `/opt/root-old` sibling bug class is closed). |
| `src/backend/i686/linker/input.rs` | **main (#854)** | Same supersession, shared helper. |
| `src/common/const_eval.rs` | **union** | main's `narrow_string_strlen` helper + my `eval_builtin_call_with_mode` policy threading + restored 3-arg wrapper; policy unit test re-extended (NUL + `-fno-builtin` tiers). |
| `src/frontend/sema/analysis.rs` | **main** | Scoped `enum_types` registry subsumes my `packed_enum_types` fallback (my side kept the old dual-map). |
| `src/frontend/sema/type_context.rs` | **main** | Same registry supersession; doc merged (md.c compatibility case preserved in prose). |
| `src/passes/constant_fold.rs` | **main** | `narrow_string_strlen` call site. |
| `backlog.md` | **main** | Their lean reconciled backlog; my completed items re-recorded below. |

Hygiene from the #854 red-team report:
- `engineering/archive/BACKLOG-2026-10-16.md` in #852 contained literal
  `<<<<<<<` blocks (1 block, lines 1/398/3700) — **restored main's clean
  archive** and deleted my duplicate `BACKLOG-2026-10-16-monolith.md`
  (byte-identical to main's `ROOT-BACKLOG-2026-10-17.md`). The conflict-
  marker gate would have rejected the branch; that catch was correct.
- Conflict-marker scan over the merged tree: clean.

## Enum layout semantics + `_Static_assert` (re-done, was lost to wipe)

Identical recipe to session 3A, re-applied on the merged tree:
- Non-packed enums follow the completed definition's **value range**
  (GCC widening): all-nonnegative ≤ `UINT_MAX` → `unsigned int`(4);
  a range needing more, or negative min with max > `INT_MAX`
  (`enum Neg { N1 = -1, N2 = 4000000000u }` → `long long`), → 8.
  Constant-4 guesses removed from `scalar_type_size_align`,
  `sizeof_type`/`alignof_type` (now routed through the CType path — the old
  `struct_union_layout` fallback made a shadowed non-packed enum report
  pointer size), sema `sizeof_type_spec`/`alignof_type_spec`, and
  `ctype_from_type_spec`. `packed_size` implements the compatible-type rule;
  struct layouts inherit it (`struct { char; enum Big; }` = 16/align 8,
  byte-identical to gcc-12).
- `_Static_assert` is now **evaluated by sema** with the full type context
  for anything beyond the parser's literal fast-path (new `StaticAssert`
  node on `ExternalDecl`/`BlockItem`; variable references still diagnosed at
  parse time per C11 6.6). A false condition
  (`_Static_assert(sizeof(enum Big) == 4)`) is a compile error, matching
  gcc; previously silently accepted.
- Parser: enum-shaped `_Alignof`/`__alignof` in ICE context delegates to
  sema (`type_spec_has_enum_tag`, recursive through pointers/arrays/typeof/
  record fields); inline enum definitions answer exactly at parse time
  (`try_enum_layout`); `struct_tag_alignments` no longer caches alignments
  for records containing enum fields (a cached 4 poisoned every later
  `_Alignof(struct WithEnum)`).
- `ctypes_compatible`: C11 6.7.2.2p4 — enum compatible with its compatible
  integer type from the registry (restores my `md_submodule_enum_compat`
  contract).

Empirical oracle agreement (gcc-12, -O0/-O1/-O2): `enumiso.c` (8/4/8 +
v=4294967296), `enumsh5.c` (2/4/2 + PA=1), `enumedge.c` (9-line
widening/packed/struct matrix) — byte-identical.

## AUD-VABI-4: one-lane FP vectors in %xmm0 (PR #853 adopted)

#853's core claim **confirmed empirically before adoption**: chained
`dadd(did(x), dadd(y,y))` segfaulted on the merged tree at -O0/-O1/-O2
(integer carrier + missing packed-result spill). Adopted their design:
- `sysv_vector_carrier`: f32x1/f64x1 → SSE (one XMM eightbyte); u64x1 →
  INTEGER (verified against gcc-12's `id_u64` = bare `ret` via %rax);
  1–4-byte integer vectors stay INTEGER.
- Backend: one-lane FP returns carry scalar F32/F64 in IR
  (`register_function_sig`, `func_return_ir_type`, `stmt_return`
  movss/movsd load), indirect-call ret classes propagate for non-struct
  returns, and small vector call results spill to an alloca when forwarded
  as arguments.
- **Additional defect found and fixed while validating their fixture**:
  `UnaryOp::Neg` on a FLOAT-element vector negated the vector's ADDRESS
  (`negq %rax; movss (%rax),%xmm0` → segfault; PR 110817 family, float
  variant). All vector negation now dispatches through the elementwise
  lowering (their fix, ported).
- Gates: their `ext_vector_one_lane_fp.c` + `ext_vector_one_lane_fp_full.c`
  fixtures (verbatim) behind `check_ext_vector_one_lane_fp.sh`
  (-O0/-O1/-O2, exact `fails=0` line; the full fixture reports on stderr).

Honest ABI-generation disclosure: gcc-12 classifies vector_size(4/8) FP
shapes MEMORY (pre-0.99-psABI); the SSE classification follows the current
psABI + GCC 16/Clang, evidenced by #853's hosted gcc-16 CI lanes (14/14).
Cross-ABI legs in the varargs gate are therefore **version-gated**
(gcc_major ≥ 13 → full v1 legs; older oracles → version-stable v4/double
legs + lccc round-trip), with the NOTE printed in the gate log.

## Red-team checklist closure (#854 report items)

1. ✅ Five main-conflict files resolved (+2 linker files git found).
2. ✅ Archived conflict block removed; marker scan clean.
3. ✅ Sysroot test determinism: main's #854 version adopted.
4. ✅ Baseline-claim fidelity: this PR body re-states the correct deltas
   (`zlib_ng_adler32` 258→255/52→50/15→14, `zstd_count` 146→145/11→7,
   `expat_xml_scan` 181→182/26→27/6→7, `hash_table` 124→126/29→30,
   `sqlite_varint` 34→35 moves — instruction/move/stack counts from
   `.github/scripts/ci-codegen-baseline.json`, not runtime claims).
5. ✅ Enum-scope gate + fixture retained (merge kept main's copy).
6. ✅ Vector-varargs gate expanded: SSE-exhaustion (8 doubles then a 32-byte
   vector via overflow), -mno-sse overflow fetch (integer-pure TU), >16-byte
   carrier-less slot, true GCC-optional skip (linker falls back to lccc),
   version-gated cross legs.
7. ✅ Kernel trust chain: a downloaded-but-sha256-mismatched tarball now
   aborts (`auto` mode no longer degrades to git on checksum mismatch —
   only an unreachable download does); the git fallback verifies the v$KVER
   tag signature (`git verify-tag`) under `LCCC_KERNEL_GIT_TAG_VERIFY=
   strict|auto|off` (default auto: verify + record `gpg-verified=` in the
   stamp, loud warning when unverifiable).

## Competing-PR verdicts (quantified)

- **#854 (merged)**: agree, adopted wholesale. It fixed a real bug class in
  my #852 (sibling-prefix strip + host-dependent test) — exactly what a
  red-team is for. Their fix is smaller and more precise than mine.
- **#853**: core claim correct (one-lane FP ABI), fix adopted after local
  reproduction of the segfault. Its synthesis of #845/#850 (varargs gate,
  enum gates, strlen NUL) is superseded by the merged #852 (which carries
  the originals plus the driver-policy and sysroot work #853 lacks).
  Duplicate-vehicle assessment: after this branch lands, #853's remaining
  unique delta is empty except AUD-VABI-4 (now included).
- **#855**: 567 files, +108k lines (~80k JSON evidence), CI state
  `unstable`, no local validation receipts ("delegated to CI"). Replaces
  `common/builtin.rs` (my per-name policy) with a per-call module and
  re-implements several #852 areas (builtin policy, SysV classification,
  sema static asserts, enum layout). Merge order matters: this branch must
  land first; #855 then needs a full re-base and per-module adjudication.
  Not adoptable at its current head: unverified build, conflicting
  equivalents of mechanisms this branch validates (e.g. its sema
  `_Static_assert` overlaps today's work with different semantics).
- **#829**: unchanged verdict (do not merge as-is; 436 files, DIRTY, no
  receipts); harvest test ideas separately.
- **#851 (merged)**: docs-only audit refresh; no overlap beyond
  `PR-AUDIT-2026-10-17.md` (merge kept main's canonical copy).

## Validation on this branch

- Build (fastbuild): 0 errors. `cargo fmt --check` clean. Clippy
  `--all-targets`: 0 warnings.
- Gates re-run green: enum-tag-scope-compat, enum-layout-semantics (new),
  ext-vector-one-lane-fp (new), ext-vector-varargs-abi (expanded),
  strlen-embedded-nul, driver-option-contract.
- `cargo test --profile fastbuild`: 5079 passed / 0 failed / 9 ignored
  (lib + all binaries). Doc-tests cannot execute in this harness (the pypi
  toolchain ships no `rustdoc`; disclosed, not worked around).
- Regression corpus: 955 passed / 9 failed / 991 total — every failure is
  the pre-existing `-m32` environment gap (`i686_*` +
  `segment_fill_copy_alias` need the wiped `/tmp/debroot` + `/tmp/i386inc`
  sysroot caches; identical set before this session's changes).
- Benchmark corpus (5 paired reps, lccc vs gcc-12, output-checked): see
  `/tmp/bench.json` + the PR body for the receipt table.

## Harness notes

- Wipe #4 destroyed /tmp, /home/user recovery artifacts and the rust
  toolchain; re-provisioned (pypi `arena-rust-toolchain` 1.97.0 + zstd shim,
  8 GiB swapfile). gcc-16.2/ICX 2026 remain unobtainable under the network
  allowlist (disclosed; the hosted CI lanes carry those contracts).

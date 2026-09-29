# S22: snapshot-loss recovery + NOTYPE COPY-vs-PLT linker fix, all backends (2026-09-29)

Sequel to the S21 notes (recovered below — the file never reached the tree:
the turn-end snapshot truncated and the S21 tree came back as a mix of file
eras). Upstream `main` is verified UNCHANGED (`93f2a43b`, PR #670): the
rebase demand is satisfied trivially — this work sits directly on latest
main (fresh clone, `ls-remote` + log agree, no newer commits anywhere on
main; the `arena/*` branches are stale August lineages).

## Forensics: the 128 MB snapshot cap truncates the workspace (rule)

`/home/user` had grown to 202 MB (old logs/patches/tarballs, two trees,
the 35 MB history bundle). Turn-end snapshots are best-effort capped
around 128 MB / 10,000 files — past the cap the restore is a PARTIAL tar
overlaid on a stale base: some files current (arm/riscv edits survived),
others silently reverted (all x86/i686/linker_common edits gone), `.git`
gone, `+x` bits stripped, toolchain/swap/multilib wiped. There is no
error anywhere; the tree just lies.

Rules adopted (non-negotiable until the cap lifts):

- Keep `/home/user` under ~100 MB at ALL times (`du -sh` before every
  snapshot). Slimming this turn: 202 MB → 22 MB (dropped superseded
  patches/logs/tarballs, the stale S20 ref tree, the old bundle; kept
  `ms178-1.patch`, the ledger, the `h2h` bench harness).
- Toolchain OUTSIDE the workspace: `RUSTUP_HOME=/opt/rustup`,
  `CARGO_HOME=/opt/cargo` (602 MB toolchain + registry must never count
  against the cap). Only the `rustup` binary + shims live in
  `~/.cargo/bin`. `/opt` is wiped between turns: reinstall per turn
  (`rustup toolchain install stable --profile minimal -c rustfmt -c
  clippy`, ~10 s on this network) and re-run `ensure_swap.sh` + the
  multilib `apt install` (both fast).
- Commit early and often (commits are kilobytes; they survive truncation
  far better than a dirty tree), and snapshot the moment a batch is green.

## Re-applied on latest main (all premises re-verified, not replayed blind)

- **Classifier (common.rs):** `rep` + short-`movs` with operands is MOVSX,
  not a string op — the CPU ignores `rep` there. New `movs_has_register_
  operand` gate keeps `rep movsl %eax,%ebx` from false-positiving a %rcx
  write (a validator false-NEGATIVE direction bug: a phantom write masks
  genuine stale reads). Bare `rep movsl/movsq` still report the counter.
  `epoch_analyzer` 9/9 (2 new Intel-contract characterization tests pin
  dest-last reading; `{att|intel}` resolves AT&T — verified in
  `x86_common.rs` — so Intel-shaped lines never occur).
- **COPY-alias sizes:** i686 `copy_groups` rep = largest member (the one
  `R_386_COPY` is sized by the rep's dynsym entry; first-seen-small
  truncated the tail) + x86-64 `copy_group_max_sizes` prepass (first-seen
  sized the shared BSS slot; later-larger overflowed). Unit tests 2/2.
- **SEC parks (FIX4): verdict SKIP, evidence kept:** the float-binop
  default is dead (all backends override), only the GEP OverAligned arm is
  live, a traits-level park would wake riscv's dormant `sec_has`
  soundness-fallback disjunct (zero audited writers there), and the SEC
  fast path fires ZERO times across -O0/-O1/-O2 + no-GLA probes (%rcx is
  single-use staging, never stage→re-query). The #3 invalidate stays; its
  comment is corrected (no memcpy path exists — all such hooks are
  overridden without the acc round-trip).

## The NOTYPE fix (new, all four linker backends)

Same-address DSO data exports with different `st_size` exposed the real
bug beneath the sizing holes: **untyped (`STT_NOTYPE`) DSO data was routed
to the PLT as if it were functions** — lccc-ld emitted zero COPY relocs
and reads returned PLT addresses (proven: `859448831 173388586512688639`
vs `1432778632 1234605616436508552`; host ld emits 2 COPYs and prints
correctly; a gcc-.o/lccc-ld matrix isolates lccc-ld).

- The reader now records whether each export lives in a `PF_X` LOAD
  segment (program headers — always present; section headers often
  stripped): `DynSymbol::in_exec_segment` (64-bit shared reader, both
  parse paths) + the i686 ELF32 equivalent.
- COPY-vs-PLT admits `STT_OBJECT` and NOTYPE-in-data, keeps NOTYPE-in-
  text on the PLT (a `lea f(%rip)` of an untyped function must still call
  real code — bfd errors that shape in a PIE; we stay correct). Wired
  through x86-64 (PC32/PLT32, absolute, R_X86_64_64, alias sweep ×2,
  `abs64_defers_to_loader`), AArch64 (3 arms + sweep ×2), RISC-V (the
  one-line site), and i686 (`register_copy_aliases` ×2).
- i686 additionally routes per reference: calls and address-takings of
  code drop `needs_copy` (`call notypefn` via PC32 previously copied code
  bytes to BSS and jumped NX). Paired predicates (`copy_data_type` /
  `is_plt_code_type`) — deliberately NOT negations, so TLS/COMMON keep
  their old routing.
- Output fidelity: COPY dynsym entries preserve the library's type (bfd
  publishes NOTYPE copies as NOTYPE, verified by readelf).
- Gates: `check_copy_alias_sizes.sh` (x86-64 PIE + i386 `-no-pie` — i686
  lccc-ld is ET_EXEC-only) and `check_linker_notype_code.sh` (functional
  42/7 on both archs + JUMP_SLOT-present/COPY-absent assertions),
  twinned into `ci_local.sh` + `ci.yml` (parity script green).

## Validation status (this turn, fresh tree)

- `cargo check --profile fastbuild`: green, zero warnings (repeated).
- Targeted lib tests: `epoch_analyzer` 9/9, `copy_group` 2/2,
  `copy_data_type` 1/1, `abs64` 2/2.
- Pending at doc time: full binary build, both new gates, displacement
  + encoder edge probes, full lib suite, `ci_local.sh --fast`, clippy.
- S21 followups carried over: SEC hit-window research (instrument before
  touching), riscv SEC activation audit if anyone parks there, `rep
  movsd` (bare) not in `REP_STRING_BASES` (unreachable — emitters only
  produce bare `rep movsb`/`rep stosb` — noted, not fixed).

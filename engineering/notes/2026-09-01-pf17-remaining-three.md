# PF-17 remaining-three (alu / huft / simd) — 2026-09-01

Rotate-ON vs GCC -O2 on the 19-name PF-17 A/B: **19 MATCH / 0 FAIL**.
Rotation stays opt-in. Snapshot after this note.

## Fixes landed this session

| File | What |
|---|---|
| `src/passes/loop_rotate.rs` | Guard D (`while (--i)` latch incoming defined in header); Guard E (skip nested loops) |
| `src/passes/univsr.rs` | Skip rotated self-loop pointer phis; unit tests on unrotated 3-block shape + `skip_rotated_self_loop_pointer_ivs` |
| `src/passes/loop_unroll.rs` | Two-block complete unroll: outside uses → last clone dest; header phi → Copy(init), not Copy(final) |
| `src/passes/backedge_pre.rs` | (prior this thread) `schedule_eprime_early` current-IR latest_dep |
| `tests/regression/loop_rotate_seq_loops.{c,env}` | Pred-label sequential loops |
| `tests/regression/loop_rotate_while_dec.{c,env}` | gzip-style `while (--i)` prefix walk |

Earlier in the same PF-17 arc: pred-label Option A `(pre_op, header_label)`.

## Do not re-debug

- Isolated alu loop1 / loop2 / alu12 (drop ys) each MATCH under rotation.
- `CCC_LOOP_ROTATE=` empty still enables the pass (`is_err()` is false). Use `env -u`.
- Disabling bepre does not fix alu/huft/simd.
- `CCC_DISABLE_PASSES=inline` does **not** stop static inlining; `__attribute__((noinline))` does.
- Tight Guard E ("cond uses a foreign header phi") is not enough after copy-prop.

## ZSTD P0

Userspace oracle MATCH on an earlier SHA. QEMU boot (`ZSTD-compressed data is corrupt`) is the gate. **Kernel tree and qemu are absent in this sandbox** — cannot close ZSTD this session.

## Next session

1. Full regression corpus with `CCC_LOOP_ROTATE=1` (474 / 0 / 11 is the bar for default-ON).
2. Sound nested-loop rotation (replace Guard E) if 9-worst / loop_patterns still show the double-jump preheader on inner loops.
3. Fetch kernel tree + qemu; ZSTD P0 boot.
4. Do not flip rotation default until (1) is green.

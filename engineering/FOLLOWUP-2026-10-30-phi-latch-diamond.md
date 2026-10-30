# Split-latch diamond phi coalescing (S81)

## Finding and root cause

`zng_insert_string_batch` in `zlib_ng_slide_hash` carried `match_head` in a
second register even though its latch value was exactly the already loaded
`h`:

```text
body:   h = load(head[hm])
        if (h != idx) { store arm } else { skip arm }
latch:  match_head = h
```

S80 emitted `movq %r12,%r11` on every iteration. The destructive-update
coalescer already had the required old-phi, folded-address, source-use and
liveness checks, but its shape gate admitted only a same-block update or a
single-predecessor chain. This direct-edge-plus-middle-arm diamond was refused
before those proofs ran.

This was not an IVSR, unrolling, or benchmark-name problem. In particular,
UNIVSR's refusal is intentional here: the 16-bit `idx` recurrence can wrap and
is not a sound general replacement for the independent pointer recurrence.

## Surgical change

`phi_update_region` is now the single shape definition consumed by both the
coalescing legality gate and the caller-saved-clobber window. It accepts:

1. the existing same-block and bounded single-predecessor-chain forms; and
2. exactly `source -> {middle, latch}`, `middle -> latch`, with two source
   successors, two latch predecessors, one middle predecessor and one middle
   successor.

A full two-middle-arm diamond, any outside predecessor, any extra successor,
or a more general CFG is rejected. The region is ordered source-to-latch and
retains the existing loop-depth policy.

All established safety proofs remain mandatory after shape recognition:
no old-phi read or deferred/folded old-phi address read in the update window,
no unsafe source use elsewhere, no source-home redefinition, no source use
before the latch copy, and no unsupported multi-def source. The apply phase
still revalidates the actual definitions and register interference.

The clobber walker now visits the middle arm even when the direct
source-to-latch edge exists. A call on either arm therefore prevents a
caller-saved inheritance; a proven callee-saved home remains eligible.

## Regression coverage

Rust tests cover the pure graph contract, the positive load/store/latch case,
an old-phi read in the middle arm, a call in that arm, an outside-predecessor
refusal and a full-diamond refusal. Results:

- 22/22 diamond-focused tests passed;
- 153/153 `backend::regalloc::` tests passed;
- the pre-existing IPA dual-classifier regression remained 1/1;
- rustfmt and `git diff --check` passed (Clippy receipt is recorded in the S81
  validation manifest).

`tests/regression/phi_coalesce_split_latch_diamond.c` adds execution coverage
for zero-trip, both if arms, an old-phi read, and a middle-arm call. GCC, S80
and S81 passed at O1/O2/O3 on x86-64 and i686. The separate S81 cross drivers
also passed against GCC with identical stdout and exit status at O1/O2/O3 on
AArch64 and RISC-V under QEMU, plus the standalone i686 driver. The official
strict AArch64 single-case runner passed 1/1.

## Measured result

The measured objects were produced by direct `-O2 -fwrapv
-ffp-contract=off -c` compilation. `benchmark_native_batched.py` retained
whole-object identity checks, fresh child state, randomized arm order and
stdout verification for every repetition. Eleven retained rounds used 165
iterations per batch. Direct paired S81/reference results on Slide were:

| comparison | median ratio | exploratory CI95 |
| --- | ---: | ---: |
| S81 / S80 | **0.9774543992** | [0.9719785956, 0.9790335418] |
| S81 / GCC 16.2 | **0.8503659416** | [0.8418807346, 0.8571916937] |
| S81 / Clang 23.1.4 | 1.1366987837 | [1.1300630362, 1.1403342361] |
| S81 / ICX 2026.2 | 1.0868121245 | [1.0847186377, 1.0907520313] |

Thus S81 is about 2.25% faster than S80 and 14.96% faster than GCC on this
workload. It does **not** beat Clang or ICX there; no best-oracle claim is
made. A short default-PASSES run was deliberately not used as acceptance
because its confidence interval crossed one.

Callgrind on the original (non-batched) executables measured 17,887,610 Ir for
S80 and 17,592,706 Ir for S81: 294,904 fewer instructions, exactly one removed
move for each of the 32,760 + 512*512 hash-insert iterations. This simulated
count supports the mechanism; it is not substituted for the native timing.

On `byte_load_reassembly`, `sqlite_vdbe_serial` and `linux_list_sort`, the
S80/S81 measured objects and linked binaries were byte-identical. The harness
therefore shared their samples rather than inventing timing deltas.

## Object inspection and remaining debt

GDB disassembly confirms the latch move is gone and the rest of the hot loop
is unchanged apart from register renaming. `objtool --stackval` with
`SRCARCH=x86` reported the same pre-existing BP/frame-pointer warnings on S80
and S81 (the insert offset moved by five bytes), not a new warning class.
`pahole` accepted the `-g` object but emitted no type records; that is not
presented as structural type evidence.

This closes only the exact split-latch diamond witness. It does not justify
arbitrary multi-arm coalescing, countdown-IV replacement, broader UNIVSR, or
loop-carried spill relaxation. Those remain separate proof-and-measurement
tasks. The remaining Slide gap to Clang/ICX and the SQLite/List-Sort gaps are
still open.

Validation receipts are retained under
`/home/user/artifacts/S81-phi-latch-diamond-validated/` and the raw benchmark
runs under `/home/user/artifacts/S78-rebased-performance/`.

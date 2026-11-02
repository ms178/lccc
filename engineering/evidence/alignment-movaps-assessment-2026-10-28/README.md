# Alignment-proven movaps/movdqa: opportunity census and runtime assessment

Question behind the deferred journal item: should the backend emit
`movaps`/`movdqa` on provably-16-aligned frame/global vector accesses instead
of `movups`/`movdqu`?

## Census (all 78 benchmark TUs, `-O2`, current output)

| operand class | count | alignment |
| --- | --- | --- |
| `disp(%rsp)` vector moves, `disp % 16 == 0` | 37 | provably aligned at canonical depth |
| `disp(%rsp)` vector moves, `disp % 16 == 8` | 12 | unaligned, must stay `movdqu` |
| RIP-relative vector moves | 4 (`movupd`, one symbol `.Lvc2`) | symbol-controlled |
| `%rbp`-based vector moves | 0 | — |
| register-register `movdqu` | 0 (already emitted `movdqa`) | — |

All 77 functions that allocate a frame keep canonical body `rsp ≡ 0 (mod 16)`
(verified by walking prologue pushes + the `subq $N,%rsp` amount against the
SysV entry invariant `rsp ≡ 8 (mod 16)`). So the sound text rule would be
`(disp + rsp_delta) % 16 == 0` for `%rsp` operands and `disp % 16 == 0` for
`%rbp`, excluding SIB/over-alignment-dance/`alloca` frames.

## Runtime measurement (this host: Intel Xeon @ 2.6 GHz)

`align_micro.c` runs identical 512 KiB 128-bit load+add+store sweeps through
`_mm_load/store_si128` (`movdqa`) vs `_mm_loadu/storeu_si128` (`movdqu`) on
64-byte-aligned buffers, 9 paired runs, minimum timing:

```
aligned(movdqa) min=71286 ns  unaligned(movdqu) min=71282 ns  ratio 0.9999
aligned(movdqa) min=71302 ns  unaligned(movdqu) min=71385 ns  ratio 1.0012
aligned(movdqa) min=71284 ns  unaligned(movdqu) min=71268 ns  ratio 0.9998
```

On modern uarches `movdqu`/`movups` decode identically to `movdqa`/`movaps`
when the effective address is aligned; the mandatory-prefix spellings
(`66` vs `F3`, or no-prefix SSE1 forms) are also equal length, so there is no
code-size delta either. The change would be spelling parity with GCC/Clang
**with no measurable runtime or size benefit**, while any provenance mistake
raises a `#GP`/SIGSEGV (the legacy packed-fold revert already hit ten such
faults). Per the metric-trap rule (never accept a change on static counts
alone), this is not pursued as a runtime optimization. The provenance work
that WOULD pay off is emitting aligned moves where it unlocks folding into
legacy packed arithmetic (`movaps` memory operand), tracked separately; the
folding path currently never fires because codegen emits `movups` + legacy
SSE arithmetic.

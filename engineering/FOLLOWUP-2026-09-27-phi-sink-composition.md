# Follow-up work — S66 (2026-09-27): sink composition + #645 audit

1. CC-O0CALL-1 (P0, OPEN): -O0 call-argument staging clobbers a live home.
   Repro `artifacts/repros/miscompile_csmith_20260945_-O0.c` (expected
   checksum 39DEBCF9, exit 0). Emitter-level fix target:
   `src/backend/x86/codegen/emit.rs`. The sink machinery does not reach it.
2. csmith differential fuzzing: binary + runtime headers wiped; rebuild per
   the recipe in the session journal, then a ≥600 s campaign at -O0/-O2
   with the CCC_VALIDATE_SSA build against gcc.
3. CI chroot mirror (`scripts/ci_ubuntu_chroot.sh`, #645): requires root;
   untestable in this sandbox (no sudo/swapon). Validate on the GH runner;
   consider a rootless bubblewrap variant so local runs can mirror too.
4. `gpr_budget()` x86-64=13: name the reserved register in a comment
   (16 − rsp − rbp − reserved = 13) so future target ports don't guess.
5. i686 copy width: the six rotation relays emit REX.W `movq` for I32 webs;
   a type-width copy-lowering rule saves 6 bytes/iter shape and may help
   decode on the SHA-256 i686 path (deferred: needs i686 wall-clock).
6. census_ab's stkref regex over-counts lea-with-displacement forms —
   align it with the gate's counter or document the divergence.
7. rot() residual deficits (gate passes; optional): bound-spill
   `cmpq %rax,%r10` (−1 insn/−stkref), epilogue xor-chain into %eax (−1).

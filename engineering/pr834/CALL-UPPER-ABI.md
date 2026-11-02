# Call-boundary upper-vector preservation

Base: merged PR834, f07f4c514460e4116a574773cd84b69d185bf65e.
One concern: final VEX promotion must preserve a callee's wide arguments.

The new fixture uses a separately GCC-compiled typed 256-bit argument witness,
not an assumption about LCCC's frontend vector ABI. Before the fix, the real
peephole pipeline changed expected `9 2 3 4` into `9 2 0 0`; the structural
companion failed too. Calls consumed upper bits that promotion had zeroed.
Caller-saved clobbers happen after argument consumption, not before it.

Calls now conservatively observe all 16 tracked upper aliases, as unresolved
transfers already do. A proven full overwrite still permits promotion. Direct,
PLT, indirect and `callq` forms are covered. The native test requires AVX on its
Linux/x86-64 runner rather than silently reporting success without execution;
the structural companion is CPU-independent.

Development library result: 4760 passed, 0 failed, 8 existing ignored. No
existing promotion-count assertion or 48-copy budget was changed. Fresh all45,
compiler-cost measurements and immutable FAST acceptance remain required.
This does not establish complete frontend interoperability for wide vectors,
and does not establish an upper-vector return ABI proof.

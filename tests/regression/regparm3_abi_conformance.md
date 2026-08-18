# regparm3_abi_conformance.c

Runtime self-consistency test for the GCC i386 `-mregparm=3` calling
convention (the Linux kernel's 16-bit boot/realmode convention).

Build & run (all opt levels must exit 0; each set bit identifies a failing case):

    lccc-x86 -m32 -O{0,1,2,s} -mregparm=3 -fno-pic tests/regression/regparm3_abi_conformance.c -o t && ./t

Covers the GCC semantics verified against gcc -m32 -mregparm=3 (14.2):
- int args in %eax/%edx/%ecx; overflow to stack KILLS remaining regs
- long long in consecutive reg PAIR (or stack + kill)
- float/double scalars on the stack WITHOUT killing GP regs
- small aggregates in ceil(size/4) consecutive regs (BLKmode rule)
- variadic functions ignore regparm entirely (all args stack)
- sret hidden pointer in %eax; callee does NOT `ret $4` under regparm
- indirect calls must not stage the target in %eax (arg 0 lives there)

History: before 2026-08-18 the callee side read register params from the
caller's stack frame (caller/callee ABI split), the never-read-store
peephole deleted ESP-relative capture stores across ESP adjustments, and
sret callees popped 4 bytes that regparm callers never pushed.

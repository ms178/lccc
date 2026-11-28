/* LCCC bundled x86gprintrin.h - the intrinsics on general registers and memory.
 *
 * GCC's header of this name gathers the scalar and system extensions (one
 * sub-header each, none of which may be included by itself); immintrin.h
 * includes it. This is the same set under the same names and signatures.
 *
 * Every function here is the instruction it is named after, written as an
 * assembler statement: none of these extensions computes something the
 * compiler would otherwise generate code for. They flush or demote a cache
 * line, read a register the kernel owns, save extended state, enter an
 * enclave. The exception is the add-with-carry family at the end, which is
 * ordinary arithmetic and is written as such.
 *
 * The predefined macro of an extension (`__CLWB__` under `-mclwb` or a
 * `-march=` that has it) promises its intrinsics; until this header existed
 * lccc refused the options and defined none of the macros.
 *
 * What GCC checks and this header does not: GCC refuses a call from a
 * function whose target lacks the extension ("target specific option
 * mismatch"). Here the call compiles and the instruction is emitted.
 */
#ifndef _X86GPRINTRIN_H_INCLUDED
#define _X86GPRINTRIN_H_INCLUDED

#if !defined(__x86_64__) && !defined(__i386__) && !defined(__i686__)
#error "x86 intrinsics (x86gprintrin.h) require an x86 target"
#endif

#include <stddef.h>
#include <ia32intrin.h>
#include <bmiintrin.h>
#include <bmi2intrin.h>
#include <lzcntintrin.h>
#include <popcntintrin.h>
#include <xsaveintrin.h>

#define __LCCC_GPR static __inline__ __attribute__((__always_inline__))
/* The low and the high half of a 64-bit mask, as EDX:EAX takes it. */
#define __LCCC_LO32(__m) ((unsigned int)(__m))
#define __LCCC_HI32(__m) ((unsigned int)((unsigned long long)(__m) >> 32))

/* === Cache lines: CLFLUSHOPT, CLWB, CLDEMOTE, CLZERO === */

/* Flush the line that holds *__A, without the ordering CLFLUSH has. */
__LCCC_GPR void
_mm_clflushopt(void *__A)
{
    __asm__ __volatile__("clflushopt %0" : : "m"(*(volatile char *)__A));
}

/* Write the line back and keep it in the cache. */
__LCCC_GPR void
_mm_clwb(void *__A)
{
    __asm__ __volatile__("clwb %0" : : "m"(*(volatile char *)__A));
}

/* Hint: move the line to a cache further from this core. */
__LCCC_GPR void
_cldemote(void *__A)
{
    __asm__ __volatile__("cldemote %0" : : "m"(*(volatile char *)__A));
}

/* Zero the whole line that holds *__I (the address goes in rAX). */
__LCCC_GPR void
_mm_clzero(void *__I)
{
    __asm__ __volatile__("clzero" : : "a"(__I) : "memory");
}

/* WBINVD and WBNOINVD write back every modified line; privileged. */
__LCCC_GPR void
_wbinvd(void)
{
    __asm__ __volatile__("wbinvd" : : : "memory");
}

__LCCC_GPR void
_wbnoinvd(void)
{
    __asm__ __volatile__("wbnoinvd" : : : "memory");
}

/* === Non-temporal single stores: MOVDIRI, MOVDIR64B, ENQCMD === */

__LCCC_GPR void
_directstoreu_u32(void *__P, unsigned int __A)
{
    __asm__ __volatile__("movdiri %1, %0" : "=m"(*(unsigned int *)__P) : "r"(__A));
}

#ifdef __x86_64__
__LCCC_GPR void
_directstoreu_u64(void *__P, unsigned long long __A)
{
    __asm__ __volatile__("movdiri %1, %0" : "=m"(*(unsigned long long *)__P) : "r"(__A));
}
#endif

/* Copy the 64 bytes at __Q to the 64-byte-aligned __P as one write. */
__LCCC_GPR void
_movdir64b(void *__P, const void *__Q)
{
    __asm__ __volatile__("movdir64b %1, %0"
                         :
                         : "r"(__P), "m"(*(const volatile char *)__Q)
                         : "memory");
}

/* Enqueue the 64-byte command at __Q to the device at __P. The result is
 * ZF: 0 when the device took the command, 1 when it has to be retried. */
__LCCC_GPR int
_enqcmd(void *__P, const void *__Q)
{
    unsigned char __retry;
    __asm__ __volatile__("enqcmd %2, %1\n\tsetz %0"
                         : "=q"(__retry)
                         : "r"(__P), "m"(*(const volatile char *)__Q)
                         : "cc", "memory");
    return __retry;
}

__LCCC_GPR int
_enqcmds(void *__P, const void *__Q)
{
    unsigned char __retry;
    __asm__ __volatile__("enqcmds %2, %1\n\tsetz %0"
                         : "=q"(__retry)
                         : "r"(__P), "m"(*(const volatile char *)__Q)
                         : "cc", "memory");
    return __retry;
}

/* === Processor identity and state: RDPID, FSGSBASE, PTWRITE, PKU === */

/* IA32_TSC_AUX, which the kernel loads with the CPU (and node) number. The
 * operand is a full register: 64 bits in long mode, 32 otherwise. */
__LCCC_GPR unsigned int
_rdpid_u32(void)
{
    unsigned long __id;
    __asm__ __volatile__("rdpid %0" : "=r"(__id));
    return (unsigned int)__id;
}

#ifdef __x86_64__
__LCCC_GPR unsigned int
_readfsbase_u32(void)
{
    unsigned int __base;
    __asm__ __volatile__("rdfsbase %0" : "=r"(__base));
    return __base;
}

__LCCC_GPR unsigned long long
_readfsbase_u64(void)
{
    unsigned long long __base;
    __asm__ __volatile__("rdfsbase %0" : "=r"(__base));
    return __base;
}

__LCCC_GPR unsigned int
_readgsbase_u32(void)
{
    unsigned int __base;
    __asm__ __volatile__("rdgsbase %0" : "=r"(__base));
    return __base;
}

__LCCC_GPR unsigned long long
_readgsbase_u64(void)
{
    unsigned long long __base;
    __asm__ __volatile__("rdgsbase %0" : "=r"(__base));
    return __base;
}

__LCCC_GPR void
_writefsbase_u32(unsigned int __B)
{
    __asm__ __volatile__("wrfsbase %0" : : "r"(__B) : "memory");
}

__LCCC_GPR void
_writefsbase_u64(unsigned long long __B)
{
    __asm__ __volatile__("wrfsbase %0" : : "r"(__B) : "memory");
}

__LCCC_GPR void
_writegsbase_u32(unsigned int __B)
{
    __asm__ __volatile__("wrgsbase %0" : : "r"(__B) : "memory");
}

__LCCC_GPR void
_writegsbase_u64(unsigned long long __B)
{
    __asm__ __volatile__("wrgsbase %0" : : "r"(__B) : "memory");
}

/* Write a value into the Processor Trace stream. */
__LCCC_GPR void
_ptwrite64(unsigned long long __B)
{
    __asm__ __volatile__("ptwrite %0" : : "r"(__B));
}
#endif /* __x86_64__ */

__LCCC_GPR void
_ptwrite32(unsigned int __B)
{
    __asm__ __volatile__("ptwrite %0" : : "r"(__B));
}

/* The protection-key rights register. Both instructions want ECX = 0, and
 * WRPKRU wants EDX = 0 as well. */
__LCCC_GPR unsigned int
_rdpkru_u32(void)
{
    unsigned int __rights, __zero;
    __asm__ __volatile__("rdpkru" : "=a"(__rights), "=d"(__zero) : "c"(0));
    (void)__zero;
    return __rights;
}

__LCCC_GPR void
_wrpkru(unsigned int __key)
{
    __asm__ __volatile__("wrpkru" : : "a"(__key), "c"(0), "d"(0) : "memory");
}

/* === Serialization and history: SERIALIZE, HRESET === */

__LCCC_GPR void
_serialize(void)
{
    __asm__ __volatile__("serialize" : : : "memory");
}

/* Reset the prediction history EAX selects. */
__LCCC_GPR void
_hreset(unsigned int __EAX)
{
    __asm__ __volatile__("hreset $0" : : "a"(__EAX));
}

/* === Waiting: UMONITOR/UMWAIT/TPAUSE, MONITORX/MWAITX === */

__LCCC_GPR void
_umonitor(void *__A)
{
    __asm__ __volatile__("umonitor %0" : : "r"(__A));
}

/* Wait until the TSC reaches __B or the monitored range is written. The
 * result is CF: 1 when the operating system's time limit ended the wait. */
__LCCC_GPR unsigned char
_umwait(unsigned int __A, unsigned long long __B)
{
    unsigned char __limit;
    __asm__ __volatile__("umwait %1\n\tsetc %0"
                         : "=q"(__limit)
                         : "r"(__A), "a"(__LCCC_LO32(__B)), "d"(__LCCC_HI32(__B))
                         : "cc");
    return __limit;
}

__LCCC_GPR unsigned char
_tpause(unsigned int __A, unsigned long long __B)
{
    unsigned char __limit;
    __asm__ __volatile__("tpause %1\n\tsetc %0"
                         : "=q"(__limit)
                         : "r"(__A), "a"(__LCCC_LO32(__B)), "d"(__LCCC_HI32(__B))
                         : "cc");
    return __limit;
}

/* rAX = the address to monitor, ECX = extensions, EDX = hints. */
__LCCC_GPR void
_mm_monitorx(void const *__P, unsigned int __E, unsigned int __H)
{
    __asm__ __volatile__("monitorx" : : "a"(__P), "c"(__E), "d"(__H));
}

/* ECX = extensions, EAX = hints, EBX = the timeout in TSC ticks. */
__LCCC_GPR void
_mm_mwaitx(unsigned int __E, unsigned int __H, unsigned int __C)
{
    __asm__ __volatile__("mwaitx" : : "c"(__E), "a"(__H), "b"(__C));
}

/* === Transactional memory: RTM, XTEST, TSXLDTRK === */

#define _XBEGIN_STARTED (~0u)
#define _XABORT_EXPLICIT (1 << 0)
#define _XABORT_RETRY (1 << 1)
#define _XABORT_CONFLICT (1 << 2)
#define _XABORT_CAPACITY (1 << 3)
#define _XABORT_DEBUG (1 << 4)
#define _XABORT_NESTED (1 << 5)
#define _XABORT_CODE(__x) (((__x) >> 24) & 0xFF)

/* Start a transaction. _XBEGIN_STARTED when it started; when it aborts,
 * execution resumes behind the XBEGIN with the abort status in EAX, which
 * is why the status is preloaded and the fallback address is the next
 * instruction. */
__LCCC_GPR unsigned int
_xbegin(void)
{
    unsigned int __status = _XBEGIN_STARTED;
    __asm__ __volatile__("xbegin 1f\n1:" : "+a"(__status) : : "memory");
    return __status;
}

__LCCC_GPR void
_xend(void)
{
    __asm__ __volatile__("xend" : : : "memory");
}

/* The status byte is part of the instruction, so it is a constant. */
#define _xabort(__imm) __asm__ __volatile__("xabort %0" : : "i"(__imm) : "memory")

/* 1 inside a transaction (ZF is clear there), 0 outside. */
__LCCC_GPR int
_xtest(void)
{
    unsigned char __inside;
    __asm__ __volatile__("xtest\n\tsetnz %0" : "=q"(__inside) : : "cc");
    return __inside;
}

__LCCC_GPR void
_xsusldtrk(void)
{
    __asm__ __volatile__("xsusldtrk");
}

__LCCC_GPR void
_xresldtrk(void)
{
    __asm__ __volatile__("xresldtrk");
}

/* === User interrupts (long mode) === */

#ifdef __x86_64__
__LCCC_GPR void
_clui(void)
{
    __asm__ __volatile__("clui");
}

__LCCC_GPR void
_stui(void)
{
    __asm__ __volatile__("stui");
}

__LCCC_GPR void
_senduipi(unsigned long long __R)
{
    __asm__ __volatile__("senduipi %0" : : "r"(__R));
}

/* The user-interrupt flag, which TESTUI copies into CF. */
__LCCC_GPR unsigned char
_testui(void)
{
    unsigned char __enabled;
    __asm__ __volatile__("testui\n\tsetc %0" : "=q"(__enabled) : : "cc");
    return __enabled;
}
#endif

/* === Extended state: XSAVEOPT, XSAVEC, XSAVES/XRSTORS, FXSAVE/FXRSTOR === */

__LCCC_GPR void
_xsaveopt(void *__P, long long __M)
{
    __asm__ __volatile__("xsaveopt (%0)"
                         :
                         : "r"(__P), "a"(__LCCC_LO32(__M)), "d"(__LCCC_HI32(__M))
                         : "memory");
}

__LCCC_GPR void
_xsavec(void *__P, long long __M)
{
    __asm__ __volatile__("xsavec (%0)"
                         :
                         : "r"(__P), "a"(__LCCC_LO32(__M)), "d"(__LCCC_HI32(__M))
                         : "memory");
}

/* XSAVES and XRSTORS include supervisor state; privileged. */
__LCCC_GPR void
_xsaves(void *__P, long long __M)
{
    __asm__ __volatile__("xsaves (%0)"
                         :
                         : "r"(__P), "a"(__LCCC_LO32(__M)), "d"(__LCCC_HI32(__M))
                         : "memory");
}

__LCCC_GPR void
_xrstors(void *__P, long long __M)
{
    __asm__ __volatile__("xrstors (%0)"
                         :
                         : "r"(__P), "a"(__LCCC_LO32(__M)), "d"(__LCCC_HI32(__M))
                         : "memory");
}

__LCCC_GPR void
_fxsave(void *__P)
{
    __asm__ __volatile__("fxsave (%0)" : : "r"(__P) : "memory");
}

__LCCC_GPR void
_fxrstor(void *__P)
{
    __asm__ __volatile__("fxrstor (%0)" : : "r"(__P) : "memory");
}

#ifdef __x86_64__
__LCCC_GPR void
_xsaveopt64(void *__P, long long __M)
{
    __asm__ __volatile__("xsaveopt64 (%0)"
                         :
                         : "r"(__P), "a"(__LCCC_LO32(__M)), "d"(__LCCC_HI32(__M))
                         : "memory");
}

__LCCC_GPR void
_xsavec64(void *__P, long long __M)
{
    __asm__ __volatile__("xsavec64 (%0)"
                         :
                         : "r"(__P), "a"(__LCCC_LO32(__M)), "d"(__LCCC_HI32(__M))
                         : "memory");
}

__LCCC_GPR void
_xsaves64(void *__P, long long __M)
{
    __asm__ __volatile__("xsaves64 (%0)"
                         :
                         : "r"(__P), "a"(__LCCC_LO32(__M)), "d"(__LCCC_HI32(__M))
                         : "memory");
}

__LCCC_GPR void
_xrstors64(void *__P, long long __M)
{
    __asm__ __volatile__("xrstors64 (%0)"
                         :
                         : "r"(__P), "a"(__LCCC_LO32(__M)), "d"(__LCCC_HI32(__M))
                         : "memory");
}

__LCCC_GPR void
_fxsave64(void *__P)
{
    __asm__ __volatile__("fxsave64 (%0)" : : "r"(__P) : "memory");
}

__LCCC_GPR void
_fxrstor64(void *__P)
{
    __asm__ __volatile__("fxrstor64 (%0)" : : "r"(__P) : "memory");
}
#endif /* __x86_64__ */

/* === Shadow stack (CET) === */

/* RDSSP leaves its register alone where shadow stacks are off, so the
 * register starts at zero: 0 is the answer there. */
#ifdef __x86_64__
__LCCC_GPR unsigned long long
_get_ssp(void)
{
    unsigned long long __ssp = 0;
    __asm__ __volatile__("rdsspq %0" : "+r"(__ssp));
    return __ssp;
}
#else
__LCCC_GPR unsigned int
_get_ssp(void)
{
    unsigned int __ssp = 0;
    __asm__ __volatile__("rdsspd %0" : "+r"(__ssp));
    return __ssp;
}
#endif

/* Pop __B entries off the shadow stack (only the low byte counts). */
__LCCC_GPR void
_inc_ssp(unsigned int __B)
{
#ifdef __x86_64__
    __asm__ __volatile__("incsspq %0" : : "r"((unsigned long long)__B));
#else
    __asm__ __volatile__("incsspd %0" : : "r"(__B));
#endif
}

__LCCC_GPR void
_saveprevssp(void)
{
    __asm__ __volatile__("saveprevssp" : : : "memory");
}

__LCCC_GPR void
_rstorssp(void *__B)
{
    __asm__ __volatile__("rstorssp (%0)" : : "r"(__B) : "memory");
}

__LCCC_GPR void
_wrssd(unsigned int __B, void *__C)
{
    __asm__ __volatile__("wrssd %0, (%1)" : : "r"(__B), "r"(__C) : "memory");
}

__LCCC_GPR void
_wrussd(unsigned int __B, void *__C)
{
    __asm__ __volatile__("wrussd %0, (%1)" : : "r"(__B), "r"(__C) : "memory");
}

#ifdef __x86_64__
__LCCC_GPR void
_wrssq(unsigned long long __B, void *__C)
{
    __asm__ __volatile__("wrssq %0, (%1)" : : "r"(__B), "r"(__C) : "memory");
}

__LCCC_GPR void
_wrussq(unsigned long long __B, void *__C)
{
    __asm__ __volatile__("wrussq %0, (%1)" : : "r"(__B), "r"(__C) : "memory");
}
#endif

__LCCC_GPR void
_setssbsy(void)
{
    __asm__ __volatile__("setssbsy" : : : "memory");
}

__LCCC_GPR void
_clrssbsy(void *__B)
{
    __asm__ __volatile__("clrssbsy (%0)" : : "r"(__B) : "memory");
}

/* === Enclaves and platform configuration: SGX, PCONFIG ===
 *
 * One instruction per family; EAX selects the leaf function and RBX, RCX
 * and RDX carry its operands, which the caller passes as __D[0..2]. A leaf
 * reads and writes only the registers it defines, so a caller of a
 * two-operand leaf passes an array of two: the forms below touch exactly
 * the elements the leaf uses when the leaf is a constant, and all three
 * when it is not (GCC's rule, and the leaf tables are Intel's). */

#define __LCCC_LEAF_BCD(__insn, __leaf, __D, __R)                                         \
    __asm__ __volatile__(__insn                                                          \
                         : "=a"(__R), "=b"((__D)[0]), "=c"((__D)[1]), "=d"((__D)[2])     \
                         : "a"(__leaf), "b"((__D)[0]), "c"((__D)[1]), "d"((__D)[2])      \
                         : "cc", "memory")
#define __LCCC_LEAF_BC(__insn, __leaf, __D, __R)                                          \
    __asm__ __volatile__(__insn                                                          \
                         : "=a"(__R), "=b"((__D)[0]), "=c"((__D)[1])                     \
                         : "a"(__leaf), "b"((__D)[0]), "c"((__D)[1])                     \
                         : "cc", "memory")
#define __LCCC_LEAF_B(__insn, __leaf, __D, __R)                                           \
    __asm__ __volatile__(__insn                                                          \
                         : "=a"(__R), "=b"((__D)[0])                                     \
                         : "a"(__leaf), "b"((__D)[0])                                    \
                         : "cc", "memory")
#define __LCCC_LEAF_C(__insn, __leaf, __D, __R)                                           \
    __asm__ __volatile__(__insn                                                          \
                         : "=a"(__R), "=c"((__D)[1])                                     \
                         : "a"(__leaf), "c"((__D)[1])                                    \
                         : "cc", "memory")
#define __LCCC_LEAF_CD(__insn, __leaf, __D, __R)                                          \
    __asm__ __volatile__(__insn                                                          \
                         : "=a"(__R), "=c"((__D)[1]), "=d"((__D)[2])                     \
                         : "a"(__leaf), "c"((__D)[1]), "d"((__D)[2])                     \
                         : "cc", "memory")

/* ENCLS, the supervisor leaves. */
__LCCC_GPR unsigned int
_encls_u32(const unsigned int __L, size_t __D[])
{
    unsigned int __R = 0;
    if (!__builtin_constant_p(__L)) {
        __LCCC_LEAF_BCD("encls", __L, __D, __R);
        return __R;
    }
    switch (__L) {
    case 0x00: /* ECREATE */
    case 0x01: /* EADD */
    case 0x04: /* EDBGRD */
    case 0x05: /* EDBGWR */
    case 0x06: /* EEXTEND */
    case 0x0A: /* EPA */
    case 0x0D: /* EAUG */
    case 0x0E: /* EMODPR */
    case 0x0F: /* EMODT */
    case 0x10: /* ERDINFO */
        __LCCC_LEAF_BC("encls", __L, __D, __R);
        break;
    case 0x03: /* EREMOVE */
    case 0x09: /* EBLOCK */
    case 0x0C: /* ETRACK */
    case 0x11: /* ETRACKC */
        __LCCC_LEAF_C("encls", __L, __D, __R);
        break;
    default: /* EINIT, ELDB, ELDU, EWB, ELDBC, ELDUC, and leaves not yet named */
        __LCCC_LEAF_BCD("encls", __L, __D, __R);
        break;
    }
    return __R;
}

/* ENCLU, the user leaves. */
__LCCC_GPR unsigned int
_enclu_u32(const unsigned int __L, size_t __D[])
{
    unsigned int __R = 0;
    if (!__builtin_constant_p(__L)) {
        __LCCC_LEAF_BCD("enclu", __L, __D, __R);
        return __R;
    }
    switch (__L) {
    case 0x01: /* EGETKEY */
    case 0x02: /* EENTER */
    case 0x03: /* ERESUME */
    case 0x04: /* EEXIT */
    case 0x05: /* EACCEPT */
    case 0x06: /* EMODPE */
        __LCCC_LEAF_BC("enclu", __L, __D, __R);
        break;
    default: /* EREPORT, EACCEPTCOPY, and leaves not yet named */
        __LCCC_LEAF_BCD("enclu", __L, __D, __R);
        break;
    }
    return __R;
}

/* ENCLV, the leaves of a virtual machine monitor. */
__LCCC_GPR unsigned int
_enclv_u32(const unsigned int __L, size_t __D[])
{
    unsigned int __R = 0;
    if (!__builtin_constant_p(__L)) {
        __LCCC_LEAF_BCD("enclv", __L, __D, __R);
        return __R;
    }
    switch (__L) {
    case 0x00: /* EDECVIRTCHILD */
    case 0x01: /* EINCVIRTCHILD */
        __LCCC_LEAF_BC("enclv", __L, __D, __R);
        break;
    case 0x02: /* ESETCONTEXT */
        __LCCC_LEAF_CD("enclv", __L, __D, __R);
        break;
    default:
        __LCCC_LEAF_BCD("enclv", __L, __D, __R);
        break;
    }
    return __R;
}

/* PCONFIG; leaf 1 (MKTME key programming) takes its structure in RBX. */
__LCCC_GPR unsigned int
_pconfig_u32(const unsigned int __L, size_t __D[])
{
    unsigned int __R = 0;
    if (__builtin_constant_p(__L) && __L == 0x01) {
        __LCCC_LEAF_B("pconfig", __L, __D, __R);
    } else {
        __LCCC_LEAF_BCD("pconfig", __L, __D, __R);
    }
    return __R;
}

/* === Add with carry, subtract with borrow ===
 *
 * *__P = __X + __Y + carry, and the result is the carry out; a carry in is
 * any nonzero __CF. `_addcarryx_` is the same function: ADCX and ADOX are
 * two ways of encoding it for interleaved chains, not another operation.
 *
 * Written as arithmetic. lccc does not yet carry the flag from one step to
 * the next, so a chain costs about ten instructions a limb where GCC's is
 * one ADC; the results are the same. */

__LCCC_GPR unsigned char
_addcarry_u32(unsigned char __CF, unsigned int __X, unsigned int __Y, unsigned int *__P)
{
    unsigned int __sum = __X + __Y;
    unsigned int __out = __sum + (__CF != 0);
    *__P = __out;
    return (unsigned char)((__sum < __X) | (__out < __sum));
}

__LCCC_GPR unsigned char
_addcarryx_u32(unsigned char __CF, unsigned int __X, unsigned int __Y, unsigned int *__P)
{
    return _addcarry_u32(__CF, __X, __Y, __P);
}

/* *__P = __X - __Y - borrow, and the result is the borrow out. */
__LCCC_GPR unsigned char
_subborrow_u32(unsigned char __CF, unsigned int __X, unsigned int __Y, unsigned int *__P)
{
    unsigned int __diff = __X - __Y;
    unsigned int __out = __diff - (__CF != 0);
    *__P = __out;
    return (unsigned char)((__X < __Y) | (__diff < (unsigned int)(__CF != 0)));
}

#ifdef __x86_64__
__LCCC_GPR unsigned char
_addcarry_u64(unsigned char __CF, unsigned long long __X, unsigned long long __Y,
              unsigned long long *__P)
{
    unsigned long long __sum = __X + __Y;
    unsigned long long __out = __sum + (__CF != 0);
    *__P = __out;
    return (unsigned char)((__sum < __X) | (__out < __sum));
}

__LCCC_GPR unsigned char
_addcarryx_u64(unsigned char __CF, unsigned long long __X, unsigned long long __Y,
               unsigned long long *__P)
{
    return _addcarry_u64(__CF, __X, __Y, __P);
}

__LCCC_GPR unsigned char
_subborrow_u64(unsigned char __CF, unsigned long long __X, unsigned long long __Y,
               unsigned long long *__P)
{
    unsigned long long __diff = __X - __Y;
    unsigned long long __out = __diff - (__CF != 0);
    *__P = __out;
    return (unsigned char)((__X < __Y) | (__diff < (unsigned long long)(__CF != 0)));
}
#endif /* __x86_64__ */

#undef __LCCC_LEAF_BCD
#undef __LCCC_LEAF_BC
#undef __LCCC_LEAF_B
#undef __LCCC_LEAF_C
#undef __LCCC_LEAF_CD
#undef __LCCC_LO32
#undef __LCCC_HI32
#undef __LCCC_GPR

#endif /* _X86GPRINTRIN_H_INCLUDED */

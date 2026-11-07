/* Separate translation units deliberately mix atomic producers/consumers. */
#ifndef ATOMIC_INTEROP_H
#define ATOMIC_INTEROP_H

typedef unsigned int u32;
typedef unsigned long long u64;
typedef unsigned __int128 u128;

u32 interop_add32(u32 *p, u32 value);
int interop_cas32(u32 *p, u32 *expected, u32 desired);
u64 interop_xchg64(u64 *p, u64 value);
#if defined(__aarch64__)
u128 interop_add128(u128 *p, u128 value);
int interop_cas128(u128 *p, u128 *expected, u128 desired);
u128 interop_xchg128(u128 *p, u128 value);
#endif
#endif

/* SPDX-License-Identifier: LGPL-2.1-or-later */
/*
 * Workload-derived kernel: GNU C Library (glibc afd131806b),
 * sysdeps/generic/dl-new-hash.h (`_dl_new_hash`) + string/strchrnul.c SWAR
 * word-at-a-time scan (`__strchrnul`).
 *
 * Exercises dynamic-linker DT_GNU_HASH 2x-unrolled `*33` symbol hashing over
 * realistic C/POSIX/C++ symbol tables and SWAR 64-bit zero-or-target byte
 * detection (`(w - 0x0101...) & ~w & 0x8080...` + `ctzll`), verifying both
 * functions against naive byte-by-byte references.
 */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#ifndef ARENA_SIZE
#define ARENA_SIZE (1U << 16)
#endif
#ifndef SYMBOL_COUNT
#define SYMBOL_COUNT 2048U
#endif
#ifndef PASSES
#define PASSES 256U
#endif

static char sym_arena[ARENA_SIZE + 64U] __attribute__((aligned(8)));
static uint32_t sym_offsets[SYMBOL_COUNT];

__attribute__((noinline))
static uint32_t glibc_dl_new_hash(const char *str)
{
    const unsigned char *s = (const unsigned char *)str;
    unsigned int h = 5381U;
    for (;;) {
        unsigned int c0 = s[0];
        if (c0 == 0U)
            return h;
        unsigned int c1 = s[1];
        if (c1 == 0U) {
            c0 += h;
            return h * 32U + c0;
        }
        c1 += c0;
        h *= 33U * 33U;
        c1 += c0 * 32U;
        h += c1;
        s += 2;
    }
}

static uint32_t ref_dl_new_hash(const char *str)
{
    uint32_t h = 5381U;
    for (const unsigned char *s = (const unsigned char *)str; *s != 0U; s++)
        h = h * 33U + *s;
    return h;
}

static inline uint64_t repeat_byte(unsigned char c)
{
    return (uint64_t)c * 0x0101010101010101ULL;
}

static inline uint64_t has_zero_byte(uint64_t w)
{
    return (w - 0x0101010101010101ULL) & ~w & 0x8080808080808080ULL;
}

__attribute__((noinline))
static const char *glibc_swar_strchrnul(const char *str, int c_in)
{
    uintptr_t s_int = (uintptr_t)str;
    const uint64_t *word_ptr = (const uint64_t *)(s_int & ~(uintptr_t)7U);
    unsigned int shift = (unsigned int)((s_int & 7U) << 3);
    uint64_t repeated_c = repeat_byte((unsigned char)c_in);

    uint64_t word = *word_ptr;
    uint64_t mask = (has_zero_byte(word) | has_zero_byte(word ^ repeated_c)) >> shift;
    if (mask != 0ULL)
        return str + ((unsigned int)__builtin_ctzll(mask) >> 3);

    for (;;) {
        word = *++word_ptr;
        mask = has_zero_byte(word) | has_zero_byte(word ^ repeated_c);
        if (mask != 0ULL)
            return (const char *)word_ptr + ((unsigned int)__builtin_ctzll(mask) >> 3);
    }
}

static const char *ref_strchrnul(const char *s, int c_in)
{
    unsigned char target = (unsigned char)c_in;
    while (*s != '\0' && (unsigned char)*s != target)
        s++;
    return s;
}

static void init_symbols(void)
{
    static const char alphabet[] =
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_";
    uint32_t s = 0xC0FFEE42U;
    size_t pos = 0;

    for (uint32_t i = 0; i < SYMBOL_COUNT; i++) {
        s = s * 1664525U + 1013904223U;
        uint32_t len = 4U + ((s >> 16) % 44U);
        if (pos + len + 2U >= ARENA_SIZE)
            pos = 0;
        sym_offsets[i] = (uint32_t)pos;
        for (uint32_t k = 0; k < len; k++) {
            s = s * 1664525U + 1013904223U;
            sym_arena[pos + k] = alphabet[(s >> 24) % (sizeof(alphabet) - 1U)];
        }
        sym_arena[pos + len] = '\0';
        pos += len + 1U;
    }
}

int main(void)
{
    init_symbols();

    for (uint32_t i = 0; i < SYMBOL_COUNT; i++) {
        const char *sym = sym_arena + sym_offsets[i];
        if (glibc_dl_new_hash(sym) != ref_dl_new_hash(sym))
            return 1;
        int target = 'a' + (int)(i % 26U);
        if (glibc_swar_strchrnul(sym, target) != ref_strchrnul(sym, target))
            return 2;
    }

    uint64_t acc = 0;
    for (uint32_t p = 0; p < PASSES; p++) {
        int sep = '_' + (int)(p & 1U);
        for (uint32_t i = 0; i < SYMBOL_COUNT; i++) {
            const char *sym = sym_arena + sym_offsets[i];
            const char *hit = glibc_swar_strchrnul(sym, sep);
            uint32_t h = glibc_dl_new_hash(sym);
            acc = (acc * 1099511628211ULL) ^ ((uint64_t)h + (uint64_t)(hit - sym));
        }
    }

    printf("%016llx\n", (unsigned long long)acc);
    return 0;
}

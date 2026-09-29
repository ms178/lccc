/* Control flow: a dense switch that should become a jump table, a sparse
 * one that should become a decision tree, and a loop with a data-dependent
 * break.  These separate "compiler chose a table" from "compiler chose a
 * chain", which is where a jump-table lowering bug hides. */
#include <stdio.h>
#include <stdint.h>

static uint32_t dense(uint32_t x) {            /* 16 cases, no default */
    switch (x) {
    case 0:  return 0x0000u; case 1:  return 0x1111u;
    case 2:  return 0x2222u; case 3:  return 0x3333u;
    case 4:  return 0x4444u; case 5:  return 0x5555u;
    case 6:  return 0x6666u; case 7:  return 0x7777u;
    case 8:  return 0x8888u; case 9:  return 0x9999u;
    case 10: return 0xAAAAu; case 11: return 0xBBBBu;
    case 12: return 0xCCCCu; case 13: return 0xDDDDu;
    case 14: return 0xEEEEu; case 15: return 0xFFFFu;
    }
    return 0xDEADu;
}
static uint32_t sparse(uint32_t x) {           /* decision tree */
    switch (x) {
    case 1u:        return 10u;
    case 1000u:     return 20u;
    case 100000u:   return 30u;
    case 10000000u: return 40u;
    case 0xFFFFFFFFu: return 50u;
    }
    return 0u;
}
static uint32_t fallthrough(uint32_t x) {      /* fallthrough chain + loop */
    uint32_t n = 0;
    switch (x % 8) {
    case 0: n += 1;   /* fallthrough */
    case 1: n += 2;   /* fallthrough */
    case 2: n += 4;   /* fallthrough */
    case 3: n += 8;   /* fallthrough */
    case 4: n += 16;  /* fallthrough */
    case 5: n += 32;  /* fallthrough */
    case 6: n += 64;  /* fallthrough */
    case 7: n += 128;
    }
    for (uint32_t i = 0; i < (x & 7u); i++) n = n * 3u + 1u;
    return n;
}
static int search(int32_t *a, int n, int32_t key) {   /* early exit */
    for (int i = 0; i < n; i++)
        if (a[i] == key) return i;
    return -1;
}

int main(void) {
    uint32_t h = 0;
    for (uint32_t i = 0; i < 4096u; i++) {
        h = h * 31u + dense(i);
        h ^= sparse(i * 2654435761u);
        h += fallthrough(i);
    }
    printf("control %08x\n", h);

    int32_t a[1024];
    for (int i = 0; i < 1024; i++) a[i] = (int32_t)(i * 7919 - 500000);
    int64_t s = 0;
    for (int i = 0; i < 1024; i += 3) s += search(a, 1024, a[i]);
    printf("control search %lld\n", (long long)s);
    return 0;
}

/* C23 `#embed` hostile-parameter robustness (PR #739 audit item F1).
 *
 * The limit/offset parameters are user-controlled u64 values that size the
 * resource read. Without a clamp against the REAL file size, one line of
 * hostile C becomes a usize::MAX allocation (capacity-overflow panic or
 * uncapturable allocator abort). lccc clamps the read to the resource size
 * BEFORE allocating, so adversarial-but-well-formed values are neutralized:
 * this fixture embeds the 256-byte oracle file with pointer-width-maximal
 * limits/offsets and verifies the full byte list still comes out intact.
 *
 * Width-awareness: the maximal representable limit is the one that fits
 * usize on the host (u64::MAX on LP64, u32::MAX on ILP32); on a narrow
 * host a wider value is rejected with a clean "invalid embed parameter"
 * diagnostic instead — pinned separately by the CI gate
 * scripts/check_deep_nesting_robustness.py (embed crash class), which
 * accepts either outcome by host width.
 *
 * LCCC_NO_COMPARE=1: host GCC 14 cannot parse #embed. Correctness oracle
 * is the FNV-1a constant 0x4242dc5249c33625 of bytes 0..255, recorded from
 * a GCC-compiled reference array (see edg_embed_byte_oracle.c). */
#include <stdio.h>
#include <stdint.h>

/* #embed parameters are scanned literally (no macro replacement inside
 * parameter arguments), so each width spells its maximal value out. */
#if UINTPTR_MAX == 0xFFFFFFFFFFFFFFFF
static const unsigned char huge_limit[] = {
    #embed "edg_embed_bytes.bin" limit(18446744073709551615)
};
static const unsigned char tib_limit[] = {
    #embed "edg_embed_bytes.bin" limit(1099511627776)
};
/* offset beyond EOF: the skip saturates at the resource size, nothing
 * remains, and the (empty) if_empty sequence leaves an empty array. */
static const unsigned char off_eof[] = {
    #embed "edg_embed_bytes.bin" clang::offset(18446744073709551615) limit(1)
};
#else
static const unsigned char huge_limit[] = {
    #embed "edg_embed_bytes.bin" limit(4294967295)
};
static const unsigned char tib_limit[] = {
    #embed "edg_embed_bytes.bin" limit(4294967295)
};
static const unsigned char off_eof[] = {
    #embed "edg_embed_bytes.bin" clang::offset(4294967295) limit(1)
};
#endif

static uint64_t fnv1a(const unsigned char *p, unsigned long n) {
    uint64_t h = 0xcbf29ce484222325ULL;
    for (unsigned long i = 0; i < n; i++) {
        h ^= p[i];
        h *= 0x100000001b3ULL;
    }
    return h;
}

int main(void) {
    if (sizeof huge_limit != 256) return 1;
    if (fnv1a(huge_limit, 256) != 0x4242dc5249c33625ULL) return 2;
    if (sizeof tib_limit != 256) return 3;
    if (fnv1a(tib_limit, 256) != 0x4242dc5249c33625ULL) return 4;
    if (sizeof off_eof != 0) return 5;
    printf("adversarial embed limits neutralized\n");
    return 0;
}

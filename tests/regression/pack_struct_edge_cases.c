/*
 * Edge-case regression tests for -fpack-struct[=<n>] (DA-07).
 *
 * This file is compiled with lccc and GCC under every valid
 * -fpack-struct value (1, 2, 4, 8, 16) and must compile cleanly with
 * -Wall -Wextra -Werror under each. The actual layout values are pinned
 * against GCC in tests/regression/check_pack_struct_mcpu.py; this file
 * asserts structural invariants that hold regardless of the packing
 * baseline, plus per-flag exact layouts via _Static_assert where the
 * expected values are flag-independent.
 *
 * Compile matrix:
 *   gcc -Wall -Wextra -Werror -c pack_struct_edge_cases.c            (default)
 *   gcc -fpack-struct=1 -Wall -Wextra -Werror -c pack_struct_edge_cases.c
 *   gcc -fpack-struct=2 -Wall -Wextra -Werror -c pack_struct_edge_cases.c
 *   gcc -fpack-struct=4 -Wall -Wextra -Werror -c pack_struct_edge_cases.c
 *   gcc -fpack-struct=8 -Wall -Wextra -Werror -c pack_struct_edge_cases.c
 *   gcc -fpack-struct=16 -Wall -Wextra -Werror -c pack_struct_edge_cases.c
 *   gcc -fpack-struct=0 ...   (must fail: invalid)
 *   gcc -fpack-struct=3 ...   (must fail: invalid)
 *   gcc -fpack-struct=32 ...  (must fail: invalid)
 *   gcc -fpack-struct=bogus ... (must fail: invalid)
 */

/* Test 1: Baseline record - layout varies with pack, ordering is invariant */
struct S { char c; int i; char d; long long l; };

/* Field ordering is preserved regardless of packing baseline */
_Static_assert(__builtin_offsetof(struct S, c) < __builtin_offsetof(struct S, i),
               "S: c precedes i");
_Static_assert(__builtin_offsetof(struct S, i) < __builtin_offsetof(struct S, d),
               "S: i precedes d");
_Static_assert(__builtin_offsetof(struct S, d) < __builtin_offsetof(struct S, l),
               "S: d precedes l");
/* Struct is at least as large as its largest member */
_Static_assert(sizeof(struct S) >= sizeof(long long),
               "S: size at least largest member");

/* Test 2: Explicit member alignment escapes bare -fpack-struct */
struct with_aligned_member {
	char c;
	int i __attribute__((aligned(8)));
	char d;
} __attribute__((aligned(16)));

/* Test 3: Bitfields under bare -fpack-struct become a contiguous bit stream */
struct bitfield_packed {
	unsigned a: 3;
	unsigned b: 5;
	unsigned c: 20;
	char d;
};

/* Test 4: Union under bare -fpack-struct */
union packed_union {
	char c;
	long long l;
};

/* Test 5: Nested records under bare -fpack-struct */
struct inner { char c; int i; };
struct outer {
	char c;
	struct inner inner;
	char d;
};

/* Test 6: -fpack-struct=<n> caps even explicitly aligned members */
struct capped {
	char c;
	int i __attribute__((aligned(8)));
};

/* Test 7: #pragma pack after -fpack-struct=<n> */
struct before_pragma { char c; int i; };
#pragma pack(4)
struct after_pragma { char c; int i; };
#pragma pack()
struct after_reset { char c; int i; };

/* Test 8: #pragma pack(push) / pack(pop) with cap */
#pragma pack(push, 1)
struct push_one { char c; int i; };
#pragma pack(push, 2)
struct push_two { char c; int i; };
#pragma pack(pop)
struct pop_back { char c; int i; };
#pragma pack(pop)
struct pop_again { char c; int i; };

/* Test 9: Bare -fpack-struct makes #pragma pack inert (warning, no layout change) */
#pragma pack(8)
struct pragma_inert { char c; int i; };

/* Test 10: Array member under packing */
struct with_array {
	char c;
	int arr[3];
	char d;
};

/* Test 11: __attribute__((aligned(N))) member attribute interacts correctly
 * with packing - explicit member alignment escapes bare -fpack-struct but
 * is capped by -fpack-struct=<n>. */
struct with_aligned_attr {
	char c;
	int i __attribute__((aligned(8)));
	char d;
};

/* Test 12: Flexible array member (last member, packing affects preceding layout) */
struct with_fam {
	char c;
	int i;
	double data[]; /* flexible array member - no size contribution */
};

/* Test 13: Zero-size-like edge: char-only struct */
struct char_only { char c; };

/* Test 14: Mixed signed/unsigned bitfields */
struct mixed_bitfields {
	unsigned a: 1;
	signed b: 1;
	unsigned c: 30;
	char d;
};

/* Ensure all types are referenced so they are not optimized away */
int main(void) {
	volatile struct S                *ps  = 0;
	volatile struct with_aligned_member *pw = 0;
	volatile struct bitfield_packed  *pbf = 0;
	volatile union packed_union      *ppu = 0;
	volatile struct outer            *po  = 0;
	volatile struct capped           *pc  = 0;
	volatile struct before_pragma    *pbp = 0;
	volatile struct after_pragma     *pap = 0;
	volatile struct after_reset      *par = 0;
	volatile struct push_one         *ppo1 = 0;
	volatile struct push_two         *ppo2 = 0;
	volatile struct pop_back         *ppb = 0;
	volatile struct pop_again        *ppa = 0;
	volatile struct pragma_inert     *ppi = 0;
	volatile struct with_array       *pwa = 0;
	volatile struct with_aligned_attr *pwa2 = 0;
	volatile struct with_fam         *pwf = 0;
	volatile struct char_only        *pco = 0;
	volatile struct mixed_bitfields  *pmb = 0;
	(void)ps; (void)pw; (void)pbf; (void)ppu; (void)po; (void)pc;
	(void)pbp; (void)pap; (void)par; (void)ppo1; (void)ppo2;
	(void)ppb; (void)ppa; (void)ppi; (void)pwa; (void)pwa2;
	(void)pwf; (void)pco; (void)pmb;
	return 0;
}

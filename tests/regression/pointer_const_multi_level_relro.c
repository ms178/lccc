/* Regression: multi-level pointer declarators must classify by the
 * OUTERMOST pointer level only.
 *
 * ── The defect ───────────────────────────────────────────────────────────
 * Declaration::is_pointer_const() used to be set whenever a const token
 * followed ANY star of the declarator.  For
 *
 *     static short * const * gp = &inner;
 *
 * the inner const (which qualifies the pointee "short * const") leaked
 * into the flag, so both global-classification sites (global_decl.rs and
 * the static-local path in stmt.rs) treated the MUTABLE gp as a read-only
 * object.  classify_global then routed it to .data.rel.ro -- the section
 * the dynamic loader mprotects read-only after relocation under RELRO.
 * The first run-time store to gp took SIGSEGV.
 *
 * Found by Csmith seed 20260928 (differential vs GCC at -O0):
 * "static uint16_t * const *g_2865 = &g_2783;", written by func_1.
 * GCC places such globals in .data; only a genuinely const object
 * ("T *const p") belongs in .data.rel.ro under PIE.
 *
 * ── The contract ─────────────────────────────────────────────────────────
 * POINTER_CONST must mean "the const follows the LAST (identifier-
 * adjacent = outermost) star":
 *
 *     char *const p         -> read-only object  (.data.rel.ro if reloc)
 *     short *const *gp      -> writable object   (.data)
 *     int *const a, *b;     -> a read-only, b writable (no cross-declarator
 *                              leakage)
 *     const short *p        -> writable object   (const is on the pointee)
 *     short *const *const q -> read-only object  (outermost level const)
 *
 * The test below stores TO the multi-level writable objects at run time;
 * on the defective build each such store SIGSEGVs once the page was
 * RELRO-protected, so this is both an output differential and a crash
 * regression.
 *
 * Expected output: ok 1 1 1 1
 */
#include <stdio.h>

static short cell = 10;
static short cell2 = 20;

/* single-star const: genuinely read-only objects */
static short *const inner = &cell;
static short *const inner2 = &cell2;

/* const on the INNER level only: gp itself is a writable object */
static short *const *gp = &inner;

/* const on every level: genuinely read-only object */
static short *const *const cgp = &inner;

/* two declarators in one declaration: the second must not inherit the
 * first one's outer-level pointer const (cross-declarator leakage) */
static short *const first_w = &cell, *second_w = &cell2;

/* three levels; const and volatile sit in the middle only, so the
 * object gpv itself is writable */
static short *const *volatile *gpv = &gp;

/* function-scope static (exercises the stmt.rs classification path);
 * returns the address of the static local so main can store through it */
static short *const **pick(void)
{
	static short *const *local_gp = &inner;
	return &local_gp;
}

int main(void)
{
	int ok = 1;

	/* Stores to the multi-level WRITABLE objects (each faulted under the
	 * RELRO misclassification): */
	gp = &inner2;     /* store to gp itself                         */
	*pick() = &inner2;/* store to the function-scope static local   */
	gpv = &gp;        /* store to gpv itself                        */
	second_w = &cell; /* second declarator stays a writable object  */

	ok &= (gp == &inner2);
	ok &= (*gp == &cell2);
	ok &= (**gp == 20);
	ok &= (gpv == &gp);
	ok &= (second_w == &cell);
	ok &= (first_w == &cell);

	/* the read-only cgp still observes the untouched chain */
	ok &= (*cgp == &cell);
	ok &= (**cgp == 10);

	/* gp now aliases inner2, so its pointee sees the cell2 update */
	cell2 = 21;
	ok &= (**gp == 21);

	printf("%s %d %d %d %d\n", ok ? "ok" : "BAD",
	       (int)(gp == &inner2), (int)(**gp == 21),
	       (int)(gpv == &gp), (int)(second_w == &cell));
	return ok ? 0 : 1;
}

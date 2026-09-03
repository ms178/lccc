/* Regression: `T *const p` must be classified as a const object.
 *
 * ── The two different "const"s ───────────────────────────────────────────
 * For a pointer declarator, `Declaration::is_const()` describes the POINTEE:
 *
 *     const char *p;        // pointee const, p itself writable
 *     char *const p;        // p itself const, pointee writable
 *     const char *const p;  // both
 *
 * The lowering code deliberately excluded pointers from `var_is_const`
 * (`decl.is_const() && !da.is_pointer`) because `const char *p` is NOT a
 * read-only object.  But that also discarded the `*const` case, where the
 * pointer really is read-only.  The parser was dropping that information
 * entirely: `skip_cv_qualifiers()`, called after each `*`, consumed the
 * `const` token and threw it away (it already had a precedent for keeping
 * `volatile`, which is how `T *volatile p` works).
 *
 * ── Why the classification matters ───────────────────────────────────────
 * `classify_global` routes a const global whose initializer needs a
 * relocation to `.data.rel.ro` and a non-const one to `.data`.  Under -fPIE a
 * pointer initialized with the address of a string literal needs one, so
 * misclassifying it emitted an absolute R_X86_64_64 into writable `.data`,
 * which the linker turns into a run-time `.rela.dyn` entry.
 *
 * ── How it manifested ────────────────────────────────────────────────────
 * Building linux-cachymod 6.18.47, `lib/zstd/common/error_private.c` has
 *
 *     static const char* const notErrorCode = "Unspecified error code";
 *
 * inlined into the boot decompressor.  It produced a single R_X86_64_64 in
 * `.rela.data`, and the kernel's own link-time sanity check rejected the
 * image outright:
 *
 *     ld: Unexpected run-time relocations (.rela) detected!
 *
 * The decompressor is loaded and self-relocated by hand-written asm; there is
 * no dynamic loader to apply such relocations, which is why the kernel treats
 * any of them as a hard error.
 *
 * Note this is a FUNCTION-SCOPE static, lowered by `lower_local_static_decl`
 * in stmt.rs -- a separate code path from the file-scope one in
 * global_decl.rs.  Both needed the fix; a file-scope-only fix still left the
 * kernel unbuildable.
 *
 * Expected output: ok 1 2 3
 */
#include <stdio.h>

/* function-scope statics (the kernel's case) */
static const char *pick(int x)
{
	static const char *const a = "alpha";
	static const char *const b = "beta";
	return x ? a : b;
}

/* file-scope, both spellings */
static const char *const top_const_ptr = "gamma";
static const char *top_plain_ptr = "delta";

/* array of const pointers */
static const char *const table[] = { "one", "two", "three" };

int main(void)
{
	int n = 0;
	if (pick(1)[0] == 'a') n++;
	if (pick(0)[0] == 'b') n++;
	if (top_const_ptr[0] == 'g') n++;
	if (top_plain_ptr[0] == 'd') n++;
	printf("%s %d %d %d\n", n == 4 ? "ok" : "BAD",
	       (int)(table[0][0] == 'o'), (int)(table[1][0] == 't'),
	       (int)(table[2][0] == 't') + 2);
	return 0;
}

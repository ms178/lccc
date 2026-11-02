/*
 * Raw GCC vector builtins used by the kernel's NAP cpuidle governor
 * (drivers/cpuidle/governors/nap): __builtin_ia32_shufps,
 * __builtin_shufflevector, __builtin_convertvector.
 *
 * These are the BY-VALUE raw builtins (no _mm_* header wrapper): a 16-byte
 * vector result is a packed I128 value, NOT a pointer. The tests cover:
 *  - each builtin across a noinline ABI boundary (two-register return),
 *  - inline use in expressions,
 *  - NESTING one builtin's result as another's argument (this needs the
 *    spill-to-slot path in the call-argument lowering; regression: the
 *    packed I128 was passed to __floattisf and dereferenced -> SIGSEGV).
 */
typedef float v4sf __attribute__((__vector_size__(16)));
typedef int v4si __attribute__((__vector_size__(16)));

extern int printf(const char *, ...);

__attribute__((noinline)) v4sf do_movhl(v4sf a, v4sf b)
{
	/* movhlps semantics: result = { b[2], b[3], a[2], a[3] } */
	return __builtin_shufflevector(b, a, 2, 3, 6, 7);
}

__attribute__((noinline)) v4sf do_shuf(v4sf a, v4sf b)
{
	/* shufps 0x1B: result = { a[3], a[2], b[1], b[0] } */
	return __builtin_ia32_shufps(a, b, 0x1B);
}

__attribute__((noinline)) v4sf do_cvt(v4si a)
{
	return __builtin_convertvector(a, v4sf);
}

__attribute__((noinline)) v4si do_trunc(v4sf a)
{
	return __builtin_convertvector(a, v4si);
}

/* Horizontal max via shuffles — the NAP governor's reduction shape. */
__attribute__((noinline)) float hmax(v4sf v)
{
	v4sf t = __builtin_ia32_shufps(v, v, 0x4E); /* swap halves */
	v4sf m = __builtin_ia32_maxps(v, t);
	t = __builtin_ia32_shufps(m, m, 0xB1);      /* swap pairs */
	m = __builtin_ia32_maxps(m, t);
	return m[0];
}

int main(void)
{
	v4sf a = {1, 2, 3, 4}, b = {5, 6, 7, 8};

	v4sf r = do_movhl(a, b);
	printf("movhl %g %g %g %g\n", r[0], r[1], r[2], r[3]);

	v4sf s = do_shuf(a, b);
	printf("shuf %g %g %g %g\n", s[0], s[1], s[2], s[3]);

	/* Nested: builtin result as argument of another call. */
	v4sf n = do_movhl((v4sf){10, 20, 30, 40},
			  __builtin_ia32_shufps(a, b, 0x1B));
	printf("nest %g %g %g %g\n", n[0], n[1], n[2], n[3]);

	v4si i = {-2, 0, 3, 127};
	v4sf c = do_cvt(i);
	printf("cvt %g %g %g %g\n", c[0], c[1], c[2], c[3]);

	v4sf f = {1.9f, -1.9f, 100.5f, 0.0f};
	v4si t = do_trunc(f);
	printf("trunc %d %d %d %d\n", t[0], t[1], t[2], t[3]);

	/* Inline convertvector (no ABI boundary). */
	v4sf inl = __builtin_convertvector((v4si){10, -20, 30, -40}, v4sf);
	printf("inl %g %g %g %g\n", inl[0], inl[1], inl[2], inl[3]);

	v4sf h = {3.5f, 9.25f, -1.0f, 7.75f};
	printf("hmax %g\n", (double)hmax(h));

	return 0;
}

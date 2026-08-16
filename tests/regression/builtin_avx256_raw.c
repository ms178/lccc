/*
 * Raw 256-bit GCC vector builtins used by the kernel NAP governor's AVX2
 * NN kernel (nap_nn_avx2.c): __builtin_ia32_{max,min,and,cmp}ps256 and
 * __builtin_ia32_vextractf128_ps256.
 *
 * v8sf results use the pointer/sret convention (32 bytes); the vextractf128
 * result is a 16-byte vector returned BY VALUE (packed I128).
 */
typedef float v8sf __attribute__((__vector_size__(32)));
typedef float v4sf __attribute__((__vector_size__(16)));

#define V8SF_SET1(x) ((v8sf){(x), (x), (x), (x), (x), (x), (x), (x)})

extern int printf(const char *, ...);

__attribute__((noinline)) v8sf clamp8(v8sf v, v8sf lo, v8sf hi)
{
	return __builtin_ia32_maxps256(__builtin_ia32_minps256(v, hi), lo);
}

__attribute__((noinline)) v4sf hi128(v8sf v)
{
	return __builtin_ia32_vextractf128_ps256(v, 1);
}

__attribute__((noinline)) v8sf mask_ge(v8sf a, v8sf b)
{
	/* cmpps imm 0x0D = GE (ordered, signaling); mask AND with 1.0f. */
	v8sf m = __builtin_ia32_cmpps256(a, b, 0x0D);
	return __builtin_ia32_andps256(m, V8SF_SET1(1.0f));
}

int main(void)
{
	int i;
	v8sf v = {-3, 0.5f, 2, 9, -1, 0.25f, 1.5f, 7};

	v8sf r = clamp8(v, V8SF_SET1(0.0f), V8SF_SET1(1.0f));
	printf("clamp8");
	for (i = 0; i < 8; i++)
		printf(" %g", r[i]);
	printf("\n");

	v4sf h = hi128(v);
	printf("hi128 %g %g %g %g\n", h[0], h[1], h[2], h[3]);

	v8sf g = mask_ge(v, V8SF_SET1(1.0f));
	printf("ge");
	for (i = 0; i < 8; i++)
		printf(" %g", g[i]);
	printf("\n");

	return 0;
}

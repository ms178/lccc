/*
 * Compile-time folding of __builtin_strcmp/strncmp/memcmp on string
 * literals. The kernel's vruntime_cmp MACRO (CachyMod fair.c) relies on
 * this fold: an if/else ladder over __builtin_strcmp(OP_STR, "<") whose
 * dead branch calls an UNDEFINED function (__BUILD_BUG_vruntime_cmp).
 * Without the fold the dead call survives into codegen and vmlinux fails
 * to link.
 *
 * NOTE: the kernel idiom is a MACRO, so both strcmp arguments are string
 * literals at the point of lowering — that is exactly what lccc folds.
 * (An inline function with a char* parameter is NOT folded; GCC only
 * manages that via post-inline optimization, which no kernel code
 * depends on.)
 *
 * The runtime checks also pin the fold's SEMANTICS to glibc/GCC: sign of
 * the first differing byte, strncmp length cutoff, memcmp byte count.
 */
extern int printf(const char *, ...);

/* Deliberately undefined: any surviving call breaks the link. */
extern void __BUILD_BUG_strcmp_fold(void);

/* The kernel's exact idiom shape (vruntime_cmp, kernel/sched/fair.c). */
#define op_class(OP_STR) ({					\
	int __res = 0;						\
	if (!__builtin_strcmp(OP_STR, "<"))			\
		__res = 1;					\
	else if (!__builtin_strcmp(OP_STR, "<="))		\
		__res = 2;					\
	else if (!__builtin_strcmp(OP_STR, ">"))		\
		__res = 3;					\
	else if (!__builtin_strcmp(OP_STR, ">="))		\
		__res = 4;					\
	else							\
		__BUILD_BUG_strcmp_fold();			\
	__res;							\
})

int main(void)
{
	/* Every branch of the dead-code idiom must fold away. */
	printf("ops %d %d %d %d\n",
	       op_class("<"), op_class("<="), op_class(">"), op_class(">="));

	printf("strcmp %d %d %d\n",
	       __builtin_strcmp("a", "a"),
	       __builtin_strcmp("a", "b") < 0,
	       __builtin_strcmp("b", "a") > 0);

	/* Prefix equality vs. length cutoff. */
	printf("strncmp %d %d %d\n",
	       __builtin_strncmp("abc", "abd", 2),
	       __builtin_strncmp("abc", "abd", 3) < 0,
	       __builtin_strncmp("ab", "abc", 5) < 0);

	printf("memcmp %d %d\n",
	       __builtin_memcmp("ab", "ab", 2),
	       __builtin_memcmp("ab", "ac", 2) < 0);

	return 0;
}

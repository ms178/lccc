#  define SPACED 1
# define TABS	2
#ifdef SPACED
spaced_ok
#endif
#define EMPTY_BODY
#define WITH_SPACE 
x EMPTY_BODY y WITH_SPACE z
#undef SPACED
#ifdef SPACED
bad
#else
undef_ok
#endif
#define FN (a)
FN
#define F2(a) a
F2
(1)

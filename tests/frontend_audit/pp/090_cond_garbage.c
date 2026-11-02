#if 0
this is not valid C at all ' " \
#if nested
#endif
#else
real_code
#endif
#ifdef NOPE
nope
#elif 1
elif_taken
#else
else_taken
#endif
#if 0
#elif 0
#elif 1
third
#endif

#if -1 < 0u
signed_lt_unsigned
#endif
#if -1 < 0
ok_neg
#endif
#if (0x7fffffffffffffff + 1) < 0
wrap_i64
#endif
#if 0xffffffffffffffffu + 1 == 0
wrap_u64
#endif
#if 1 ? 2 : (1 / 0)
ternary_short
#endif
#if (3, 0)
comma
#else
comma_zero
#endif
#if 'A' == 65
char_const
#endif
#if 'ab' == 24930
multi_char
#endif
#if defined(__x86_64__) && !defined(nope)
defined_ok
#endif
#if 1u - 2u > 0
unsigned_wrap
#endif
#if (-1 >> 1) == -1
arith_shift
#endif
#if ~0 == -1
bitnot
#endif
#if 0x10 % 7 == 2 && 0x10 / 7 == 2
divmod
#endif
#if !0 && !!1
logic
#endif
#if (1 << 63) < 0
shift_i64
#endif
#if UNDEFINED_MACRO + 1 == 1
undef_is_zero
#endif

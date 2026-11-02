/* LCCC standard header <stdckdint.h> (ISO C23 7.20). */
#ifndef _STDCKDINT_H
#define _STDCKDINT_H

#define __STDC_VERSION_STDCKDINT_H__ 202311L

#define ckd_add(r, a, b) ((_Bool)__builtin_add_overflow((a), (b), (r)))
#define ckd_sub(r, a, b) ((_Bool)__builtin_sub_overflow((a), (b), (r)))
#define ckd_mul(r, a, b) ((_Bool)__builtin_mul_overflow((a), (b), (r)))

#endif /* _STDCKDINT_H */

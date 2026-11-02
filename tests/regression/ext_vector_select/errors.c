/* Invalid ext-vector element selections. check_ext_vector_swizzle_diagnostics.sh
 * requires LCCC's `line:col: error: message` list for this file to equal Clang
 * 23.1.4's (the type spelling after "exceeds type" aside: LCCC has no typedef
 * names in its types). It never compiles: kept out of the runner's glob. */
typedef int I1 __attribute__((ext_vector_type(1)));
typedef int I2 __attribute__((ext_vector_type(2)));
typedef int I4 __attribute__((ext_vector_type(4)));
typedef _Bool B8 __attribute__((ext_vector_type(8)));

int names(I4 v, I1 one, B8 b) {
    int a = v.xr;      /* mixed xyzw and rgba */
    a += v.q;          /* unknown letter */
    a += v.s;          /* bare s */
    a += v.sx;         /* s without hex digits */
    a += v.s1x;        /* a letter among hex digits */
    a += v.x0;         /* xyzw followed by a digit */
    a += v.hix;        /* hi/lo/even/odd stand alone */
    a += v.Hi;         /* case-sensitive */
    a += v  .  zq;     /* caret: operator + 1 + index (19:15, a space) */
    a += v.s4;         /* index beyond the vector */
    a += v.w + one.y;  /* index beyond a one-lane vector */
    a += b.s1;         /* _Bool vectors have no element selection */
    a += b.x;
    a += b.lo.x;
    return a;
}

void assignments(I4 v, I4 *p, I2 t) {
    v.xx = t;          /* duplicate lane in the target */
    v.xx.y = 1;        /* duplicate lane in an inner selection */
    v.xxyy.yz = t;     /* ... even though the composed lanes differ */
    v.xx.y++;
    v.xx += t;
    --p->yy.x;
    p->wzyx.xy.x++;    /* fine */
    v.xy.z = 1;        /* exceeds the inner selection's 2 lanes */
}

int *addresses(I4 *p) {
    int *q = &p->x;    /* address of a vector element */
    return q ? &p->wz.y : q;
}

void steps(I4 v, I4 *p) {
    v++;               /* Clang has no ++/-- on a vector in C */
    --p->zw;           /* ... nor on a multi-lane selection */
    v.x++;             /* fine: one lane is a scalar */
    p->wzyx.y--;       /* fine */
}

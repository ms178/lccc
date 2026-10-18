#!/usr/bin/env python3
"""Generate matrix.c (the vector-extension function matrix) and driver.c
(the lccc-vs-GCC runtime differential driver) for check_vector_ext_codegen.sh.
Run from this directory; both outputs are checked in."""
import itertools
elems={'c':'char','s':'short','i':'int','l':'long','f':'float','d':'double'}
ops={'add':'a+b','sub':'a-b','mul':'a*b','and':'a&b','or':'a|b','xor':'a^b','shl':'a<<b','shr':'a>>b','neg':'-a','not':'~a','eq':'a==b','lt':'a<b','div':'a/b'}
out=[]
for e,ct in elems.items():
    for w in (16,32):
        t=f"v{w}{e}"
        out.append(f"typedef {ct} {t} __attribute__((vector_size({w})));")
        for o,ex in ops.items():
            if e in 'fd' and o in ('and','or','xor','shl','shr','not'): continue
            if e not in 'fd' and o=='div': continue
            if o in ('eq','lt'):
                rt = t if e not in 'fd' else (f"v{w}i" if e=='f' else f"v{w}l")
                out.append(f"{rt} {o}_{t}({t} a,{t} b){{return {ex};}}")
            elif o=='min':
                if e in 'fd': continue
                out.append(f"{t} {o}_{t}({t} a,{t} b){{return {ex};}}")
            else:
                out.append(f"{t} {o}_{t}({t} a,{t} b){{return {ex};}}")
        out.append(f"void loop_{t}({t}*r,const {t}*a,const {t}*b,int n){{for(int i=0;i<n;i++)r[i]=a[i]*b[i]+r[i];}}")
        out.append(f"{t} splat_{t}({ct} x){{return ({t}){{}}+x;}}")
        out.append(f"{ct} lane2_{t}({t} a){{return a[2];}}")
        out.append(f"{t} setlane_{t}({t} a,{ct} x){{a[1]=x;return a;}}")
        out.append(f"{t} ld_{t}(const {ct}*p){{{t} v; __builtin_memcpy(&v,p,sizeof v); return v;}}")
        out.append(f"void st_{t}({ct}*p,{t} v){{__builtin_memcpy(p,&v,sizeof v);}}")
        out.append(f"{t} cond_{t}({t} a,{t} b,int c){{return c?a+b:a-b;}}")
        out.append(f"{t} acc_{t}(const {t}*p,int n){{{t} s={{}};for(int i=0;i<n;i++)s+=p[i];return s;}}")
# struct wrapping
out.append("struct S8f { v32f v; }; struct S8f sadd(struct S8f a, struct S8f b){ a.v += b.v; return a; }")
out.append("v32f call8(v32f a); v32f viacall(v32f a, v32f b){ return call8(a+b) * b; }")


# Hand-written edge cases (ABI + evaluation-order + aliasing), appended
# verbatim; the driver below exercises each one.
EXTRA = r"""
/* scalar + vector register interleave: s in %xmm0, a in %ymm1, b in %ymm2 */
static v32d c_dmix_helper(v32d t){ return t*t-t; }
static void c_touch(v32f*p){ (*p)[1]=(*p)[0]+1.0f; }
v32f mix_sab(float s, v32f a, v32f b){ return a*s + b; }
v16f mix_sab4(float s, v16f a, v16f b){ return a*s + b; }
/* scalar splat on the right / left */
v32i splat_r(v32i a){ return a + 3; }
v32f splat_l(v32f a){ return 2.0f * a; }
/* nine vector args: the ninth has no XMM register and arrives in memory */
v32f nine(v32f a,v32f b,v32f c,v32f d,v32f e,v32f f,v32f g,v32f h,v32f i){ return a+b+c+d+e+f+g+h+i; }
v16i nine4(v16i a,v16i b,v16i c,v16i d,v16i e,v16i f,v16i g,v16i h,v16i i){ return a+b+c+d+e+f+g+h+i; }
/* identity / parameter passthrough */
v32f ident(v32f a){ return a; }
v16d ident2(v16d a){ return a; }
/* whole-object assignment through pointers, including self-assignment */
void asg(v32f*r,const v32f*a){ *r=*a; }
void asg_self(v32f*r){ *r=*r; }
void asg4(v16i*r,const v16i*a){ *r=*a; }
/* overlapping union members: the load must precede the store */
union U { v32f v; v16f h[2]; };
void asg_union(union U*u){ u->v = u->v + u->v; }
/* ternary with lvalue arms, with side-effecting arms (one arm evaluated),
   nested, and with a call arm */
v32f tern_lv(v32f a, v32f b, int c){ return c ? a : b; }
int tern_count;
static v32f bump(v32f a){ tern_count++; return a + 1.0f; }
v32f tern_se(v32f a, v32f b, int c){ return c ? bump(a) : bump(b); }
v32f tern_nest(v32f a, v32f b, int c){ return c > 1 ? a + b : c ? a - b : a * b; }
v32f tern_assign(v32f a, v32f b, int c){ v32f r; r = c ? a : b; return r + 1.0f; }
/* sign of zero through negate / subtract */
v32f negz(v32f a){ return -a; }
v16d negz2(v16d a){ return -a; }
/* struct with a vector member passed/returned by value */
struct S4i { v16i v; int tag; };
struct S4i s4i_add(struct S4i a, v16i b){ a.v += b; a.tag++; return a; }
/* compound assign on a dereferenced pointer and on an array element */
void cas_ptr(v32f*p, v32f b){ *p *= b; }
void cas_arr(v16i*p, v16i b, int i){ p[i] ^= b; }
/* comparison mask feeding a blend by hand */
v32i sel_mask(v32f a, v32f b, v32i x, v32i y){ v32i m = a < b; return (m & x) | (~m & y); }
/* FP contraction shapes (vecfma): fused under FMA3 exactly where GCC fuses
   (every use of the product an absorbing add/sub in the same block), so the
   single-rounding results stay bit-identical across the differential */
v32f fma_f8(v32f a, v32f b, v32f c){ return a*b + c; }
v32f fma_f8r(v32f a, v32f b, v32f c){ return c + a*b; }
v32f fnma_f8(v32f a, v32f b, v32f c){ return c - a*b; }
v32f fms_f8(v32f a, v32f b, v32f c){ return a*b - c; }
v32d fma_d4(v32d a, v32d b, v32d c){ return a*b + c; }
v32d fnma_d4(v32d a, v32d b, v32d c){ return c - a*b; }
v32d fms_d4(v32d a, v32d b, v32d c){ return a*b - c; }
v16f fma_f4(v16f a, v16f b, v16f c){ return a*b + c; }
v16f fnma_f4(v16f a, v16f b, v16f c){ return c - a*b; }
v16f fms_f4(v16f a, v16f b, v16f c){ return a*b - c; }
v16d fma_d2(v16d a, v16d b, v16d c){ return a*b + c; }
v16d fnma_d2(v16d a, v16d b, v16d c){ return c - a*b; }
v32f fma_sq(v32f a, v32f c){ return a*a + c; }
v16f fma_sq4(v16f a, v16f c){ return a*a + c; }
v32f fma_self(v32f a, v32f b){ return a*b + a; }
v32f fma_chain(v32f a, v32f b, v32f c, v32f d){ return (a*b + c)*d + a; }
v32f fma_two(v32f a, v32f b, v32f c, v32f d){ v32f m = a*b; return (m + c) - (m - d); }
v32f fma_nouse(v32f a, v32f b, v32f c){ v32f m = a*b; return (m + c) * m; }
void fma_loop(v32f*r,const v32f*a,const v32f*b,const v32f*c,int n){ for(int i=0;i<n;i++) r[i]=a[i]*b[i]+c[i]; }
void fma_acc(v32d*r,const v32d*a,const v32d*b,int n){ v32d s={}; for(int i=0;i<n;i++) s = s + a[i]*b[i]; *r=s; }
/* FMA3 destructive handoff in EVERY operand position (regalloc
   `allocate_vector_registers_destructive`): the dying operand that becomes
   the destination may be the multiplicand (p0/p1), the addend (p2 → 231
   form), aliased (sq/al/al2: the 132 form reads both sources before the
   write), chained, used twice, or still live afterwards (live: no handoff
   allowed, the temporary must survive the FMA) — at all four widths */
#define HOFF(T,N) \
T N##_p0(T a,T b,T c){ T t=a+a; return t*b+c; } \
T N##_p1(T a,T b,T c){ T t=b+b; return a*t+c; } \
T N##_p2(T a,T b,T c){ T t=c+c; return a*b+t; } \
T N##_p1s(T a,T b,T c){ T t=b+b; return a*t-c; } \
T N##_p2s(T a,T b,T c){ T t=c+c; return t-a*b; } \
T N##_p2n(T a,T b,T c){ T t=c+c; return -(a*b)-t; } \
T N##_sq(T a,T b,T c){ T t=a+b; return t*t+c; } \
T N##_al(T a,T b,T c){ T t=a+b; return t*b+t; } \
T N##_al2(T a,T b,T c){ T t=a+b; return b*t+t; } \
T N##_ch(T a,T b,T c,T d){ T t=a*b+c; return t*d+t; } \
T N##_ch2(T a,T b,T c,T d){ T t=a*b+c; T u=t*d+a; return u*t+b; } \
T N##_two(T a,T b,T c,T d){ T t=a+b; return (t*c+d)+(t*d-c); } \
T N##_ret0(T a,T b,T c){ return c*b+a; } \
T N##_ret1(T a,T b,T c){ return b*c+a; } \
T N##_live(T a,T b,T c,T*o){ T t=a+a; *o=t*b+c; return t; }
HOFF(v16f,ho_f4) HOFF(v16d,ho_d2) HOFF(v32f,ho_f8) HOFF(v32d,ho_d4)
/* Control-flow / lifetime stress for the register-promotion pass (`vecssa`)
   and the param-preload prologue: phis across calls, loop-carried
   accumulators with and without calls in the body, nested ternaries,
   switch, goto loops, two live vectors across a call, long-double-free
   mixes, lane access after packed ops, conditional assignment, struct
   wrapping, address escape, volatile temporaries, compare chains,
   self-assignment, dead stores, per-lane rewrites and a zero-initialised
   temporary read through every lane. */
void sink(void);
v32f cf_phi_call(v32f a, v32f b, int c){ v32f r = c ? a+b : a-b; sink(); return r*r; }
v32f cf_loop_acc(const v32f*a, int n){ v32f s={}; for(int i=0;i<n;i++) s+=a[i]; return s; }
v32f cf_loop_acc_call(const v32f*a, int n){ v32f s={}; for(int i=0;i<n;i++){ s+=a[i]; if(i&1) sink(); } return s; }
v32f cf_nested_tern(v32f a, v32f b, int c){ return c>1 ? a*b : (c ? a+b : a-b); }
v32f cf_sw(v32f a, v32f b, int c){ v32f r=a; switch(c&3){ case 0: r=a+b; break; case 1: r=a-b; break; case 2: r=a*b; break; default: r=b; } return r+a; }
v32f cf_goto_loop(const v32f*a, int n){ v32f s=a[0]; int i=1; top: if(i<n){ s=s*a[i]+a[0]; i++; goto top; } return s; }
v32f cf_two_lives(v32f a, v32f b){ v32f x=a+b, y=a-b; sink(); return x*y; }
v32l cf_l_phi(v32l a, v32l b, int c){ v32l r = c ? a&b : a|b; return r^a; }
v32d cf_d_mix(v32d a, v32d b, double s){ v32d t=a*s; return c_dmix_helper(t)+b; }
float cf_lane_after(v32f a, v32f b){ v32f t=a*b+a; return t[0]+t[7]+t[3]; }
v32f cf_cond_assign(v32f a, v32f b, int c){ v32f r=a; if(c) r=b*a; return r+b; }
struct W8 { v32f v; int tag; };
v32f cf_wrap_phi(struct W8 w, v32f b, int c){ v32f r = c ? w.v : b; return r-w.v; }
v32f cf_addr_escape(v32f a, v32f b){ v32f t=a+b; c_touch(&t); return t*b; }
v32f cf_volatile_t(v32f a, v32f b){ volatile v32f t=a*b; v32f u=t; return u+a; }
v32i cf_cmp_chain(v32f a, v32f b, v32f c){ return (a<b)&(b<c); }
v32f cf_self_assign(v32f a){ a=a; a=a*a; a=a; return a; }
v32f cf_dead_store(v32f a, v32f b){ v32f t=a+b; t=a-b; t=t*b; return t; }
v32f cf_all_lanes(v32f a){ v32f t=a*a; for(int i=0;i<8;i++) t[i]=t[i]+(float)i; return t; }
v32f cf_uninit(v32f a){ v32f t={}; return t+a; }
/* Per-lane variable shifts (`vpsllv*`/`vpsrlv*`/`vpsravd`): unsigned lanes
   (logical right), non-constant scalar counts (splat), a count vector of
   the other signedness, 64-bit lanes (no `vpsravq` below AVX-512VL: the
   signed 64-bit right shift stays scalar), and 16-byte shapes. */
typedef unsigned v32u __attribute__((vector_size(32)));
typedef unsigned long v32ul __attribute__((vector_size(32)));
typedef unsigned v16u __attribute__((vector_size(16)));
v32u sh_vv_u(v32u a, v32u b){ return a >> b; }
v32ul sh_vv_ul(v32ul a, v32ul b){ return a >> b; }
v16u sh_vv_u4(v16u a, v16u b){ return a >> b; }
v32i sh_vs_shl(v32i a, int n){ return a << n; }
v32i sh_vs_sar(v32i a, int n){ return a >> n; }
v32u sh_vs_shr(v32u a, unsigned n){ return a >> n; }
v32l sh_vs_shl_l(v32l a, long n){ return a << n; }
v32ul sh_vs_shr_ul(v32ul a, int n){ return a >> n; }
v32i sh_mixsign(v32i a, v32u b){ return a << b; }
v32u sh_mixsign2(v32u a, v32i b){ return a >> b; }
v32i sh_chain(v32i a, v32i b, v32i c){ return (a << b) >> c; }
v32i sh_then_add(v32i a, v32i b){ return (a << b) + a; }
v16i sh4_vs_sar(v16i a, int n){ return a >> n; }
v16l sh4_vv_shl_l(v16l a, v16l b){ return a << b; }
v16i sh4_chain(v16i a, v16i b, v16i c){ return (a << b) + (a >> c); }
/* Integer compare predicates the operator grid does not reach: 64-bit
   lanes (vpcmpeqq/vpcmpgtq), unsigned orderings (sign-bias remap), le/ge
   (min-equality or inversion), ne, the VEX.128 twins, and a compare mask
   consumed by a select. */
typedef unsigned long v16ul __attribute__((vector_size(16)));
typedef short v16s_ __attribute__((vector_size(16)));
v32l cq_lt_s(v32l a, v32l b){ return a < b; }
v32l cq_le_s(v32l a, v32l b){ return a <= b; }
v32l cq_gt_s(v32l a, v32l b){ return a > b; }
v32l cq_ne_s(v32l a, v32l b){ return a != b; }
v32l cq_lt_u(v32ul a, v32ul b){ return a < b; }
v32l cq_le_u(v32ul a, v32ul b){ return a <= b; }
v32l cq_ge_u(v32ul a, v32ul b){ return a >= b; }
v32l cq_eq_u(v32ul a, v32ul b){ return a == b; }
v16l cq4_lt_s(v16l a, v16l b){ return a < b; }
v16l cq4_ne_s(v16l a, v16l b){ return a != b; }
v16l cq4_lt_u(v16ul a, v16ul b){ return a < b; }
v16l cq4_le_u(v16ul a, v16ul b){ return a <= b; }
v16i cd4_le_s(v16i a, v16i b){ return a <= b; }
v16i cd4_ne_s(v16i a, v16i b){ return a != b; }
v16i cd4_lt_u(v16u a, v16u b){ return a < b; }
v16i cd4_le_u(v16u a, v16u b){ return a <= b; }
v16s_ cw4_ne_s(v16s_ a, v16s_ b){ return a != b; }
v16c cb4_le_s(v16c a, v16c b){ return a <= b; }
v32l cq_sel(v32l a, v32l b, v32l x, v32l y){ v32l m = a < b; return (m & x) | (~m & y); }
"""
out.append(EXTRA)
open('matrix.c','w').write("\n".join(out)+"\n")

import re, sys
elems={'c':'char','s':'short','i':'int','l':'long','f':'float','d':'double'}
ops={'add':2,'sub':2,'mul':2,'and':2,'or':2,'xor':2,'shl':2,'shr':2,'neg':1,'not':1,'eq':2,'lt':2,'div':2}
src=open('matrix.c').read().split('/* scalar + vector register interleave')[0]
fns=[]
for line in src.splitlines():
    for m in re.finditer(r'(struct \w+|\w+) (\w+)\(([^()]*)\)\s*\{', line):
        fns.append((m.group(1), m.group(2), m.group(3)))
out=[]
out.append('#include <stdio.h>\n#include <string.h>\n#include <stdint.h>\n')
for e,ct in elems.items():
    for w in (16,32):
        out.append(f"typedef {ct} v{w}{e} __attribute__((vector_size({w})));")
out.append("struct S8f { v32f v; };")
# declarations
for pfx in ('G_','L_'):
    for rt,name,args in fns:
        out.append(f"{rt} {pfx}{name}({args});")
    out.append(f"v32f {pfx}call8(v32f a){{ return a*a - a; }}")
    out.append(f"static int {pfx}sink_count; void {pfx}sink(void){{ {pfx}sink_count++; }}")
out.append(r'''
static uint64_t seed=0x9E3779B97F4A7C15ull;
static uint64_t rnd(void){ seed^=seed<<13; seed^=seed>>7; seed^=seed<<17; return seed; }
static void fill(void*p,int n,int kind){ unsigned char*b=p; for(int i=0;i<n;i++) b[i]=(unsigned char)rnd(); (void)kind; }
static int fails=0, total=0;
static void check(const char*n,const void*a,const void*b,int sz){ total++; if(memcmp(a,b,sz)){ fails++; printf("MISMATCH %s\n",n);} }
''')
body=[]
for rt,name,args in fns:
    t=name.split('_',1)[1] if '_' in name else None
    kind=name.split('_')[0]
    elem = None
    if t and t.startswith('v'):
        w=int(t[1:-1]); e=t[-1]; elem=elems[e]; lanes=w//{'c':1,'s':2,'i':4,'l':8,'f':4,'d':8}[e]
    if kind in ops and ops[kind]==2:
        body.append(f'{{ {t} a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);')
        if kind in ('shl','shr'):
            body.append(f'  for(int i=0;i<{lanes};i++) b[i]=({elem})(((unsigned)b[i])%{8*w//lanes});')
        if kind=='div':
            body.append(f'  for(int i=0;i<{lanes};i++) {{ if(b[i]==0) b[i]=3; if(a[i]==({elem})(1ull<<{8*w//lanes-1}) && b[i]==({elem})-1) b[i]=2; }}')
        if elem in ('float','double'):
            body.append(f'  for(int i=0;i<{lanes};i++) {{ a[i]=({elem})(int)(a[i]*0+((int)rnd()%1000))/7.0; b[i]=({elem})((int)rnd()%1000)/3.0; if(b[i]==0)b[i]=1; }}')
        body.append(f'  {rt} r1=G_{name}(a,b), r2=L_{name}(a,b); check("{name}",&r1,&r2,sizeof r1); }}')
    elif kind in ops and ops[kind]==1:
        body.append(f'{{ {t} a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0);')
        if elem in ('float','double'):
            body.append(f'  for(int i=0;i<{lanes};i++) a[i]=({elem})((int)rnd()%1000)/7.0;')
        body.append(f'  {rt} r1=G_{name}(a,b), r2=L_{name}(a,b); check("{name}",&r1,&r2,sizeof r1); }}')
    elif kind=='loop':
        body.append(f'{{ {t} r1[9],r2[9],a[9],b[9]; fill(a,sizeof a,0); fill(b,sizeof b,0); fill(r1,sizeof r1,0); memcpy(r2,r1,sizeof r1);')
        if elem in ('float','double'):
            body.append(f'  for(int k=0;k<9;k++) for(int i=0;i<{lanes};i++) {{ a[k][i]=({elem})((int)rnd()%100); b[k][i]=({elem})((int)rnd()%100); r1[k][i]=r2[k][i]=({elem})((int)rnd()%100); }}')
        body.append(f'  G_{name}(r1,a,b,9); L_{name}(r2,a,b,9); check("{name}",r1,r2,sizeof r1); }}')
    elif kind=='splat':
        body.append(f'{{ {elem} x=({elem})(rnd()%200); {t} r1=G_{name}(x), r2=L_{name}(x); check("{name}",&r1,&r2,sizeof r1); }}')
    elif kind=='lane2':
        body.append(f'{{ {t} a; fill(&a,sizeof a,0); for(int i=0;i<{lanes};i++) a[i]=({elem})(int)(rnd()%200); {elem} r1=G_{name}(a), r2=L_{name}(a); check("{name}",&r1,&r2,sizeof r1); }}')
    elif kind=='setlane':
        body.append(f'{{ {t} a; fill(&a,sizeof a,0); for(int i=0;i<{lanes};i++) a[i]=({elem})(int)(rnd()%200); {elem} x=({elem})(rnd()%200); {t} r1=G_{name}(a,x), r2=L_{name}(a,x); check("{name}",&r1,&r2,sizeof r1); }}')
    elif kind=='ld':
        body.append(f'{{ {elem} p[{lanes}]; fill(p,sizeof p,0); {t} r1=G_{name}(p), r2=L_{name}(p); check("{name}",&r1,&r2,sizeof r1); }}')
    elif kind=='st':
        body.append(f'{{ {t} v; fill(&v,sizeof v,0); {elem} p1[{lanes}],p2[{lanes}]; G_{name}(p1,v); L_{name}(p2,v); check("{name}",p1,p2,sizeof p1); }}')
    elif kind=='cond':
        body.append(f'{{ {t} a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<{lanes};i++) {{ a[i]=({elem})(int)(rnd()%200); b[i]=({elem})(int)(rnd()%200); }} for(int c=0;c<2;c++) {{ {t} r1=G_{name}(a,b,c), r2=L_{name}(a,b,c); check("{name}",&r1,&r2,sizeof r1); }} }}')
    elif kind=='acc':
        body.append(f'{{ {t} p[6]; for(int k=0;k<6;k++) for(int i=0;i<{lanes};i++) p[k][i]=({elem})(int)(rnd()%100); for(int n=0;n<=6;n+=3) {{ {t} r1=G_{name}(p,n), r2=L_{name}(p,n); check("{name}",&r1,&r2,sizeof r1); }} }}')
    elif name=='sadd':
        body.append('{ struct S8f a,b; for(int i=0;i<8;i++){a.v[i]=(float)(int)(rnd()%100); b.v[i]=(float)(int)(rnd()%100);} struct S8f r1=G_sadd(a,b), r2=L_sadd(a,b); check("sadd",&r1,&r2,sizeof r1); }')
    elif name=='viacall':
        body.append('{ v32f a,b; for(int i=0;i<8;i++){a[i]=(float)(int)(rnd()%100); b[i]=(float)(int)(rnd()%100);} v32f r1=G_viacall(a,b), r2=L_viacall(a,b); check("viacall",&r1,&r2,sizeof r1); }')
    else:
        print("unhandled", name, file=sys.stderr)
out.append(r"""
struct S4i { v16i v; int tag; };
union U { v32f v; v16f h[2]; };
v32f G_mix_sab(float,v32f,v32f); v32f L_mix_sab(float,v32f,v32f);
v16f G_mix_sab4(float,v16f,v16f); v16f L_mix_sab4(float,v16f,v16f);
v32i G_splat_r(v32i); v32i L_splat_r(v32i);
v32f G_splat_l(v32f); v32f L_splat_l(v32f);
v32f G_nine(v32f,v32f,v32f,v32f,v32f,v32f,v32f,v32f,v32f); v32f L_nine(v32f,v32f,v32f,v32f,v32f,v32f,v32f,v32f,v32f);
v16i G_nine4(v16i,v16i,v16i,v16i,v16i,v16i,v16i,v16i,v16i); v16i L_nine4(v16i,v16i,v16i,v16i,v16i,v16i,v16i,v16i,v16i);
v32f G_ident(v32f); v32f L_ident(v32f);
v16d G_ident2(v16d); v16d L_ident2(v16d);
void G_asg(v32f*,const v32f*); void L_asg(v32f*,const v32f*);
void G_asg_self(v32f*); void L_asg_self(v32f*);
void G_asg4(v16i*,const v16i*); void L_asg4(v16i*,const v16i*);
void G_asg_union(union U*); void L_asg_union(union U*);
v32f G_tern_lv(v32f,v32f,int); v32f L_tern_lv(v32f,v32f,int);
extern int G_tern_count, L_tern_count;
v32f G_tern_se(v32f,v32f,int); v32f L_tern_se(v32f,v32f,int);
v32f G_tern_nest(v32f,v32f,int); v32f L_tern_nest(v32f,v32f,int);
v32f G_tern_assign(v32f,v32f,int); v32f L_tern_assign(v32f,v32f,int);
v32f G_negz(v32f); v32f L_negz(v32f);
v16d G_negz2(v16d); v16d L_negz2(v16d);
struct S4i G_s4i_add(struct S4i, v16i); struct S4i L_s4i_add(struct S4i, v16i);
void G_cas_ptr(v32f*,v32f); void L_cas_ptr(v32f*,v32f);
void G_cas_arr(v16i*,v16i,int); void L_cas_arr(v16i*,v16i,int);
v32i G_sel_mask(v32f,v32f,v32i,v32i); v32i L_sel_mask(v32f,v32f,v32i,v32i);
#define FMA3(T,N) T G_##N(T,T,T); T L_##N(T,T,T);
#define FMA2(T,N) T G_##N(T,T); T L_##N(T,T);
FMA3(v32f,fma_f8) FMA3(v32f,fma_f8r) FMA3(v32f,fnma_f8) FMA3(v32f,fms_f8)
FMA3(v32d,fma_d4) FMA3(v32d,fnma_d4) FMA3(v32d,fms_d4)
FMA3(v16f,fma_f4) FMA3(v16f,fnma_f4) FMA3(v16f,fms_f4)
FMA3(v16d,fma_d2) FMA3(v16d,fnma_d2)
FMA2(v32f,fma_sq) FMA2(v16f,fma_sq4) FMA2(v32f,fma_self) FMA3(v32f,fma_nouse)
v32f G_fma_chain(v32f,v32f,v32f,v32f); v32f L_fma_chain(v32f,v32f,v32f,v32f);
v32f G_fma_two(v32f,v32f,v32f,v32f); v32f L_fma_two(v32f,v32f,v32f,v32f);
void G_fma_loop(v32f*,const v32f*,const v32f*,const v32f*,int); void L_fma_loop(v32f*,const v32f*,const v32f*,const v32f*,int);
void G_fma_acc(v32d*,const v32d*,const v32d*,int); void L_fma_acc(v32d*,const v32d*,const v32d*,int);
struct W8 { v32f v; int tag; };
typedef unsigned v32u __attribute__((vector_size(32)));
typedef unsigned long v32ul __attribute__((vector_size(32)));
typedef unsigned v16u __attribute__((vector_size(16)));
#define SH(pfx) v32u pfx##sh_vv_u(v32u,v32u); v32ul pfx##sh_vv_ul(v32ul,v32ul); v16u pfx##sh_vv_u4(v16u,v16u); \
  v32i pfx##sh_vs_shl(v32i,int); v32i pfx##sh_vs_sar(v32i,int); v32u pfx##sh_vs_shr(v32u,unsigned); \
  v32l pfx##sh_vs_shl_l(v32l,long); v32ul pfx##sh_vs_shr_ul(v32ul,int); v32i pfx##sh_mixsign(v32i,v32u); \
  v32u pfx##sh_mixsign2(v32u,v32i); v32i pfx##sh_chain(v32i,v32i,v32i); v32i pfx##sh_then_add(v32i,v32i); \
  v16i pfx##sh4_vs_sar(v16i,int); v16l pfx##sh4_vv_shl_l(v16l,v16l); v16i pfx##sh4_chain(v16i,v16i,v16i);
SH(G_) SH(L_)
typedef unsigned long v16ul __attribute__((vector_size(16)));
typedef short v16s_ __attribute__((vector_size(16)));
#define CQ(pfx) v32l pfx##cq_lt_s(v32l,v32l); v32l pfx##cq_le_s(v32l,v32l); v32l pfx##cq_gt_s(v32l,v32l); v32l pfx##cq_ne_s(v32l,v32l); \
  v32l pfx##cq_lt_u(v32ul,v32ul); v32l pfx##cq_le_u(v32ul,v32ul); v32l pfx##cq_ge_u(v32ul,v32ul); v32l pfx##cq_eq_u(v32ul,v32ul); \
  v16l pfx##cq4_lt_s(v16l,v16l); v16l pfx##cq4_ne_s(v16l,v16l); v16l pfx##cq4_lt_u(v16ul,v16ul); v16l pfx##cq4_le_u(v16ul,v16ul); \
  v16i pfx##cd4_le_s(v16i,v16i); v16i pfx##cd4_ne_s(v16i,v16i); v16i pfx##cd4_lt_u(v16u,v16u); v16i pfx##cd4_le_u(v16u,v16u); \
  v16s_ pfx##cw4_ne_s(v16s_,v16s_); v16c pfx##cb4_le_s(v16c,v16c); v32l pfx##cq_sel(v32l,v32l,v32l,v32l);
CQ(G_) CQ(L_)
#define CF(pfx) v32f pfx##cf_phi_call(v32f,v32f,int); v32f pfx##cf_loop_acc(const v32f*,int); v32f pfx##cf_loop_acc_call(const v32f*,int); \
  v32f pfx##cf_nested_tern(v32f,v32f,int); v32f pfx##cf_sw(v32f,v32f,int); v32f pfx##cf_goto_loop(const v32f*,int); v32f pfx##cf_two_lives(v32f,v32f); \
  v32l pfx##cf_l_phi(v32l,v32l,int); v32d pfx##cf_d_mix(v32d,v32d,double); float pfx##cf_lane_after(v32f,v32f); v32f pfx##cf_cond_assign(v32f,v32f,int); \
  v32f pfx##cf_wrap_phi(struct W8,v32f,int); v32f pfx##cf_addr_escape(v32f,v32f); v32f pfx##cf_volatile_t(v32f,v32f); v32i pfx##cf_cmp_chain(v32f,v32f,v32f); \
  v32f pfx##cf_self_assign(v32f); v32f pfx##cf_dead_store(v32f,v32f); v32f pfx##cf_all_lanes(v32f); v32f pfx##cf_uninit(v32f);
CF(G_) CF(L_)
#define HOFFD(T,N) FMA3(T,N##_p0) FMA3(T,N##_p1) FMA3(T,N##_p2) FMA3(T,N##_p1s) FMA3(T,N##_p2s) FMA3(T,N##_p2n) \
  FMA3(T,N##_sq) FMA3(T,N##_al) FMA3(T,N##_al2) FMA3(T,N##_ret0) FMA3(T,N##_ret1) \
  T G_##N##_ch(T,T,T,T); T L_##N##_ch(T,T,T,T); T G_##N##_ch2(T,T,T,T); T L_##N##_ch2(T,T,T,T); \
  T G_##N##_two(T,T,T,T); T L_##N##_two(T,T,T,T); T G_##N##_live(T,T,T,T*); T L_##N##_live(T,T,T,T*);
HOFFD(v16f,ho_f4) HOFFD(v16d,ho_d2) HOFFD(v32f,ho_f8) HOFFD(v32d,ho_d4)
/* inexact lane values (odd / 7): the product rounds, so a fused and a
   separately rounded evaluation differ in the last bit */
static void fillx8(v32f*v){ for(int i=0;i<8;i++) (*v)[i]=(float)((int)(rnd()%20000)-10000)/7.0f; }
static void fillx4(v16f*v){ for(int i=0;i<4;i++) (*v)[i]=(float)((int)(rnd()%20000)-10000)/7.0f; }
static void fillxd4(v32d*v){ for(int i=0;i<4;i++) (*v)[i]=(double)((int)(rnd()%20000)-10000)/7.0; }
static void fillxd2(v16d*v){ for(int i=0;i<2;i++) (*v)[i]=(double)((int)(rnd()%20000)-10000)/7.0; }
#define CHK3(T,N,FILL) { T a,b,c; FILL(&a); FILL(&b); FILL(&c); T r1=G_##N(a,b,c), r2=L_##N(a,b,c); check(#N,&r1,&r2,sizeof r1); }
#define CHK2(T,N,FILL) { T a,b; FILL(&a); FILL(&b); T r1=G_##N(a,b), r2=L_##N(a,b); check(#N,&r1,&r2,sizeof r1); }
static void fma_checks(void){
  for(int rep=0;rep<4;rep++){
  CHK3(v32f,fma_f8,fillx8) CHK3(v32f,fma_f8r,fillx8) CHK3(v32f,fnma_f8,fillx8) CHK3(v32f,fms_f8,fillx8)
  CHK3(v32d,fma_d4,fillxd4) CHK3(v32d,fnma_d4,fillxd4) CHK3(v32d,fms_d4,fillxd4)
  CHK3(v16f,fma_f4,fillx4) CHK3(v16f,fnma_f4,fillx4) CHK3(v16f,fms_f4,fillx4)
  CHK3(v16d,fma_d2,fillxd2) CHK3(v16d,fnma_d2,fillxd2)
  CHK2(v32f,fma_sq,fillx8) CHK2(v16f,fma_sq4,fillx4) CHK2(v32f,fma_self,fillx8) CHK3(v32f,fma_nouse,fillx8)
  { v32f a,b,c,d; fillx8(&a); fillx8(&b); fillx8(&c); fillx8(&d); v32f r1=G_fma_chain(a,b,c,d), r2=L_fma_chain(a,b,c,d); check("fma_chain",&r1,&r2,sizeof r1);
    r1=G_fma_two(a,b,c,d); r2=L_fma_two(a,b,c,d); check("fma_two",&r1,&r2,sizeof r1); }
  { v32f a[5],b[5],c[5],r1[5],r2[5]; for(int k=0;k<5;k++){ fillx8(&a[k]); fillx8(&b[k]); fillx8(&c[k]); } G_fma_loop(r1,a,b,c,5); L_fma_loop(r2,a,b,c,5); check("fma_loop",r1,r2,sizeof r1); }
  { v32d a[5],b[5],r1,r2; for(int k=0;k<5;k++){ fillxd4(&a[k]); fillxd4(&b[k]); } G_fma_acc(&r1,a,b,5); L_fma_acc(&r2,a,b,5); check("fma_acc",&r1,&r2,sizeof r1); }
  }
}
#define CHK4(T,N,FILL) { T a,b,c,d; FILL(&a); FILL(&b); FILL(&c); FILL(&d); T r1=G_##N(a,b,c,d), r2=L_##N(a,b,c,d); check(#N,&r1,&r2,sizeof r1); }
#define CHKL(T,N,FILL) { T a,b,c,o1,o2; FILL(&a); FILL(&b); FILL(&c); T r1=G_##N(a,b,c,&o1), r2=L_##N(a,b,c,&o2); check(#N,&r1,&r2,sizeof r1); check(#N ".o",&o1,&o2,sizeof o1); }
#define HOFFC(T,N,FILL) CHK3(T,N##_p0,FILL) CHK3(T,N##_p1,FILL) CHK3(T,N##_p2,FILL) CHK3(T,N##_p1s,FILL) CHK3(T,N##_p2s,FILL) CHK3(T,N##_p2n,FILL) \
  CHK3(T,N##_sq,FILL) CHK3(T,N##_al,FILL) CHK3(T,N##_al2,FILL) CHK3(T,N##_ret0,FILL) CHK3(T,N##_ret1,FILL) \
  CHK4(T,N##_ch,FILL) CHK4(T,N##_ch2,FILL) CHK4(T,N##_two,FILL) CHKL(T,N##_live,FILL)
static void fillf8(v32f*v);
#define CFEQ(N,...) { v32f r1=G_##N(__VA_ARGS__), r2=L_##N(__VA_ARGS__); check(#N,&r1,&r2,sizeof r1); }
static void cf_checks(void){
  v32f arr[7]; for(int k=0;k<7;k++) fillf8(&arr[k]);
  v32f a,b,c; fillf8(&a); fillf8(&b); fillf8(&c);
  for(int n=0;n<=6;n++){ CFEQ(cf_loop_acc,arr,n) CFEQ(cf_loop_acc_call,arr,n) CFEQ(cf_goto_loop,arr,n<1?1:n) }
  for(int k=0;k<4;k++){
    CFEQ(cf_phi_call,a,b,k) CFEQ(cf_nested_tern,a,b,k) CFEQ(cf_sw,a,b,k) CFEQ(cf_cond_assign,a,b,k)
    { v32l x,y; fill(&x,sizeof x,0); fill(&y,sizeof y,0); v32l r1=G_cf_l_phi(x,y,k), r2=L_cf_l_phi(x,y,k); check("cf_l_phi",&r1,&r2,sizeof r1); }
    { struct W8 w; w.v=a; w.tag=k; CFEQ(cf_wrap_phi,w,b,k) }
  }
  CFEQ(cf_two_lives,a,b) CFEQ(cf_addr_escape,a,b) CFEQ(cf_volatile_t,a,b) CFEQ(cf_self_assign,a) CFEQ(cf_dead_store,a,b) CFEQ(cf_all_lanes,a) CFEQ(cf_uninit,a)
  { v32d x,y; fillxd4(&x); fillxd4(&y); double s=(double)(rnd()%40)/8.0; v32d r1=G_cf_d_mix(x,y,s), r2=L_cf_d_mix(x,y,s); check("cf_d_mix",&r1,&r2,sizeof r1); }
  { float r1=G_cf_lane_after(a,b), r2=L_cf_lane_after(a,b); check("cf_lane_after",&r1,&r2,sizeof r1); }
  { v32i r1=G_cf_cmp_chain(a,b,c), r2=L_cf_cmp_chain(a,b,c); check("cf_cmp_chain",&r1,&r2,sizeof r1); }
  check("cf_sink_count",&G_sink_count,&L_sink_count,sizeof(int));
}
/* counts reduced modulo the lane width: wider counts are undefined for
   GNU vector shifts exactly as for the scalar lanes */
#define SHVV(T,N,W) { T a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(unsigned i=0;i<sizeof a/sizeof a[0];i++) b[i]=(b[i]&(W-1)); \
  T r1=G_##N(a,b), r2=L_##N(a,b); check(#N,&r1,&r2,sizeof r1); }
#define SHVS(T,N,CT,W) { T a; fill(&a,sizeof a,0); CT n=(CT)(rnd()%W); T r1=G_##N(a,n), r2=L_##N(a,n); check(#N,&r1,&r2,sizeof r1); }
static void shift_checks(void){
  for(int rep=0;rep<4;rep++){
    SHVV(v32u,sh_vv_u,32) SHVV(v32ul,sh_vv_ul,64) SHVV(v16u,sh_vv_u4,32) SHVV(v32i,sh_then_add,32)
    SHVS(v32i,sh_vs_shl,int,32) SHVS(v32i,sh_vs_sar,int,32) SHVS(v32u,sh_vs_shr,unsigned,32)
    SHVS(v32l,sh_vs_shl_l,long,64) SHVS(v32ul,sh_vs_shr_ul,int,64)
    SHVS(v16i,sh4_vs_sar,int,32) SHVV(v16l,sh4_vv_shl_l,64)
    { v16i a,b,c; fill(&a,sizeof a,0); fill(&b,sizeof b,0); fill(&c,sizeof c,0); for(int i=0;i<4;i++){ b[i]&=31; c[i]&=31; } v16i r1=G_sh4_chain(a,b,c), r2=L_sh4_chain(a,b,c); check("sh4_chain",&r1,&r2,sizeof r1); }
    { v32i a; v32u b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<8;i++) b[i]&=31; v32i r1=G_sh_mixsign(a,b), r2=L_sh_mixsign(a,b); check("sh_mixsign",&r1,&r2,sizeof r1); }
    { v32u a; v32i b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); for(int i=0;i<8;i++) b[i]&=31; v32u r1=G_sh_mixsign2(a,b), r2=L_sh_mixsign2(a,b); check("sh_mixsign2",&r1,&r2,sizeof r1); }
    { v32i a,b,c; fill(&a,sizeof a,0); fill(&b,sizeof b,0); fill(&c,sizeof c,0); for(int i=0;i<8;i++){ b[i]&=31; c[i]&=31; } v32i r1=G_sh_chain(a,b,c), r2=L_sh_chain(a,b,c); check("sh_chain",&r1,&r2,sizeof r1); }
  }
}
/* compare operands: random lanes, with some lanes forced equal and some to
   the sign-bit boundary so eq/le/unsigned-order paths are all exercised */
#define CMPV(T,N,AT) { AT a,b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); \
  for(unsigned i=0;i<sizeof a/sizeof a[0];i++){ if(i%3==0) b[i]=a[i]; if(i%4==1) a[i]=(__typeof__(a[0]))((__typeof__(a[0]))1<<(sizeof a[0]*8-1)); } \
  T r1=G_##N(a,b), r2=L_##N(a,b); check(#N,&r1,&r2,sizeof r1); }
static void cmp64_checks(void){
  for(int rep=0;rep<4;rep++){
    CMPV(v32l,cq_lt_s,v32l) CMPV(v32l,cq_le_s,v32l) CMPV(v32l,cq_gt_s,v32l) CMPV(v32l,cq_ne_s,v32l)
    CMPV(v32l,cq_lt_u,v32ul) CMPV(v32l,cq_le_u,v32ul) CMPV(v32l,cq_ge_u,v32ul) CMPV(v32l,cq_eq_u,v32ul)
    CMPV(v16l,cq4_lt_s,v16l) CMPV(v16l,cq4_ne_s,v16l) CMPV(v16l,cq4_lt_u,v16ul) CMPV(v16l,cq4_le_u,v16ul)
    CMPV(v16i,cd4_le_s,v16i) CMPV(v16i,cd4_ne_s,v16i) CMPV(v16i,cd4_lt_u,v16u) CMPV(v16i,cd4_le_u,v16u)
    CMPV(v16s_,cw4_ne_s,v16s_) CMPV(v16c,cb4_le_s,v16c)
    { v32l a,b,x,y; fill(&a,sizeof a,0); fill(&b,sizeof b,0); fill(&x,sizeof x,0); fill(&y,sizeof y,0); b[1]=a[1];
      v32l r1=G_cq_sel(a,b,x,y), r2=L_cq_sel(a,b,x,y); check("cq_sel",&r1,&r2,sizeof r1); }
  }
}
static void handoff_checks(void){
  for(int rep=0;rep<3;rep++){
    HOFFC(v16f,ho_f4,fillx4) HOFFC(v16d,ho_d2,fillxd2) HOFFC(v32f,ho_f8,fillx8) HOFFC(v32d,ho_d4,fillxd4)
  }
}
static void fillf8(v32f*v){ for(int i=0;i<8;i++) (*v)[i]=(float)((int)(rnd()%2000)-1000)/8.0f; }
static void fillf4(v16f*v){ for(int i=0;i<4;i++) (*v)[i]=(float)((int)(rnd()%2000)-1000)/8.0f; }
static void filld2(v16d*v){ for(int i=0;i<2;i++) (*v)[i]=(double)((int)(rnd()%2000)-1000)/8.0; }
static void extra_checks(void){
  { v32f a,b; fillf8(&a); fillf8(&b); float s=(float)(rnd()%50)/4.0f; v32f r1=G_mix_sab(s,a,b), r2=L_mix_sab(s,a,b); check("mix_sab",&r1,&r2,sizeof r1); }
  { v16f a,b; fillf4(&a); fillf4(&b); float s=(float)(rnd()%50)/4.0f; v16f r1=G_mix_sab4(s,a,b), r2=L_mix_sab4(s,a,b); check("mix_sab4",&r1,&r2,sizeof r1); }
  { v32i a; fill(&a,sizeof a,0); v32i r1=G_splat_r(a), r2=L_splat_r(a); check("splat_r",&r1,&r2,sizeof r1); }
  { v32f a; fillf8(&a); v32f r1=G_splat_l(a), r2=L_splat_l(a); check("splat_l",&r1,&r2,sizeof r1); }
  { v32f v[9]; for(int k=0;k<9;k++) fillf8(&v[k]); v32f r1=G_nine(v[0],v[1],v[2],v[3],v[4],v[5],v[6],v[7],v[8]), r2=L_nine(v[0],v[1],v[2],v[3],v[4],v[5],v[6],v[7],v[8]); check("nine",&r1,&r2,sizeof r1); }
  { v16i v[9]; fill(v,sizeof v,0); v16i r1=G_nine4(v[0],v[1],v[2],v[3],v[4],v[5],v[6],v[7],v[8]), r2=L_nine4(v[0],v[1],v[2],v[3],v[4],v[5],v[6],v[7],v[8]); check("nine4",&r1,&r2,sizeof r1); }
  { v32f a; fillf8(&a); v32f r1=G_ident(a), r2=L_ident(a); check("ident",&r1,&r2,sizeof r1); }
  { v16d a; filld2(&a); v16d r1=G_ident2(a), r2=L_ident2(a); check("ident2",&r1,&r2,sizeof r1); }
  { v32f a,r1,r2; fillf8(&a); fillf8(&r1); r2=r1; G_asg(&r1,&a); L_asg(&r2,&a); check("asg",&r1,&r2,sizeof r1); G_asg_self(&r1); L_asg_self(&r2); check("asg_self",&r1,&r2,sizeof r1); }
  { v16i a,r1,r2; fill(&a,sizeof a,0); fill(&r1,sizeof r1,0); r2=r1; G_asg4(&r1,&a); L_asg4(&r2,&a); check("asg4",&r1,&r2,sizeof r1); }
  { union U u1,u2; fillf8(&u1.v); u2=u1; G_asg_union(&u1); L_asg_union(&u2); check("asg_union",&u1,&u2,sizeof u1); }
  { v32f a,b; fillf8(&a); fillf8(&b); for(int c=0;c<3;c++){ v32f r1=G_tern_lv(a,b,c), r2=L_tern_lv(a,b,c); check("tern_lv",&r1,&r2,sizeof r1);
      r1=G_tern_nest(a,b,c); r2=L_tern_nest(a,b,c); check("tern_nest",&r1,&r2,sizeof r1);
      r1=G_tern_assign(a,b,c); r2=L_tern_assign(a,b,c); check("tern_assign",&r1,&r2,sizeof r1);
      G_tern_count=L_tern_count=0; r1=G_tern_se(a,b,c); r2=L_tern_se(a,b,c); check("tern_se",&r1,&r2,sizeof r1); check("tern_se_count",&G_tern_count,&L_tern_count,sizeof(int)); } }
  { v32f a={0.0f,-0.0f,1.0f,-1.0f,0.0f,-0.0f,2.5f,-2.5f}; v32f r1=G_negz(a), r2=L_negz(a); check("negz",&r1,&r2,sizeof r1); }
  { v16d a={0.0,-0.0}; v16d r1=G_negz2(a), r2=L_negz2(a); check("negz2",&r1,&r2,sizeof r1); }
  { struct S4i a; v16i b; fill(&a,sizeof a,0); fill(&b,sizeof b,0); struct S4i r1=G_s4i_add(a,b), r2=L_s4i_add(a,b); check("s4i_add.v",&r1.v,&r2.v,sizeof r1.v); check("s4i_add.tag",&r1.tag,&r2.tag,sizeof r1.tag); }
  { v32f p1,p2,b; fillf8(&p1); p2=p1; fillf8(&b); G_cas_ptr(&p1,b); L_cas_ptr(&p2,b); check("cas_ptr",&p1,&p2,sizeof p1); }
  { v16i p1[3],p2[3],b; fill(p1,sizeof p1,0); memcpy(p2,p1,sizeof p1); fill(&b,sizeof b,0); G_cas_arr(p1,b,2); L_cas_arr(p2,b,2); check("cas_arr",p1,p2,sizeof p1); }
  { v32f a,b; v32i x,y; fillf8(&a); fillf8(&b); fill(&x,sizeof x,0); fill(&y,sizeof y,0); v32i r1=G_sel_mask(a,b,x,y), r2=L_sel_mask(a,b,x,y); check("sel_mask",&r1,&r2,sizeof r1); }
}
""")
out.append('int main(void){ extra_checks(); fma_checks(); handoff_checks(); cf_checks(); shift_checks(); cmp64_checks();')
out.extend(body)
out.append('printf("%d/%d checks passed\\n", total-fails, total); return fails!=0; }')
open('driver.c','w').write('\n'.join(out)+'\n')

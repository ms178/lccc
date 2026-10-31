#!/usr/bin/env python3
"""Generate the deterministic RA holdout corpus (RAP-01).

The generated translation units are intentionally not committed: this script
is the single source of truth, and its fixed ordering/content makes corpus
reproduction straightforward.  Kernels are synthetic workload shapes, not
copied third-party source.  They are a holdout: do not tune allocator knobs on
these results; use them only after selecting a change on the tuning corpus.

Usage: python3 tests/ra/gen_rastress.py [--output-dir DIR] [--list]
"""
from __future__ import annotations

import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

KERNELS: dict[str, str] = {
    "bitdecode": r'''static uint64_t kernel(uint64_t seed) {
  uint8_t bytes[256]; for (unsigned i=0;i<256;i++) bytes[i]=(uint8_t)(seed*33+i*17);
  uint64_t acc=0; unsigned bit=0, end=256*8;
  for (unsigned n=0;n<192;n++) { unsigned width=(n%13)+1, v=0;
    for (unsigned j=0;j<width && bit<end;j++,bit++) v|=((bytes[bit>>3]>>(bit&7))&1u)<<j;
    acc=(acc^(v+0x9e3779b9u))*0x100000001b3ull; }
  return acc ^ bit;
}''',
    "poly1305": r'''static uint64_t kernel(uint64_t seed) {
  uint64_t r0=(seed&0x3ffffff),r1=(seed>>3)&0x3ffff03,r2=(seed>>7)&0x3ffc0ff,r3=(seed>>11)&0x3f03fff,r4=(seed>>17)&0x00fffff;
  uint64_t h0=1,h1=2,h2=3,h3=4,h4=5;
  for(unsigned i=0;i<320;i++){ uint64_t m=(uint64_t)i*0x1ffffff;
    h0+=m; h1+=h0>>26; h0&=0x3ffffff; h2+=h1>>26; h1&=0x3ffffff;
    h3+=h2>>26; h2&=0x3ffffff; h4+=h3>>26; h3&=0x3ffffff;
    h0*=r0; h1=h1*r0+h0*r1; h2=h2*r0+h1*r1+h0*r2;
    h3=h3*r0+h2*r1+h1*r2+h0*r3; h4=h4*r0+h3*r1+h2*r2+h1*r3+h0*r4;
    h0^=h4>>23; h1^=h0>>19; h2^=h1>>17; h3^=h2>>13; h4^=h3>>11;
  } return h0^h1^h2^h3^h4;
}''',
    "fft4_stage": r'''static uint64_t kernel(uint64_t seed) {
  double re[256],im[256]; for(unsigned i=0;i<256;i++){re[i]=(double)((i+seed%31)%97)/97.;im[i]=(double)((i*7+3)%89)/89.;}
  unsigned n4=64; for(unsigned k=0;k<n4;k++){ unsigned a=k,b=k+n4,c=k+2*n4,d=k+3*n4;
    double ar=re[a],ai=im[a],br=re[b],bi=im[b],cr=re[c],ci=im[c],dr=re[d],di=im[d];
    double t0=ar+cr,t1=ai+ci,t2=br+dr,t3=bi+di;
    re[a]=t0+t2; im[a]=t1+t3; re[b]=t0-t2; im[b]=t1-t3;
    re[c]=ar-cr+bi-di; im[c]=ai-ci+dr-br; re[d]=ar-cr-bi+di; im[d]=ai-ci-dr+br;
  } uint64_t x=0; for(unsigned i=0;i<256;i++) x=x*33+(uint64_t)(re[i]*1000+im[i]*2000); return x;
}''',
    "fe_mul": r'''static uint64_t kernel(uint64_t seed) {
  int64_t a[10],b[10],h[10]; for(int i=0;i<10;i++){a[i]=(int64_t)(seed+i*7919)%1000000;b[i]=(int64_t)(seed/3+i*104729)%1000000;h[i]=0;}
  for(int round=0;round<90;round++){ for(int i=0;i<10;i++){ int64_t x=0;
      for(int j=0;j<10;j++) x += a[j]*b[(i+j)%10]*(j==9?19:1);
      h[i]=(x+(int64_t)round*17) % 1000000007; }
    for(int i=0;i<10;i++){a[i]=h[(i+1)%10];b[i]=(b[i]+h[i])%1000003;} }
  uint64_t s=0;for(int i=0;i<10;i++)s=s*131+(uint64_t)h[i];return s;
}''',
    "blake2b": r'''static uint64_t kernel(uint64_t seed) {
  uint64_t v[16]; for(unsigned i=0;i<16;i++)v[i]=seed+i*0x9e3779b97f4a7c15ull;
  for(unsigned r=0;r<96;r++){ unsigned a=r&15,b=(r+4)&15,c=(r+8)&15,d=(r+12)&15;
    v[a]+=v[b]+(r*0x1000193u); v[d]^=v[a];v[d]=(v[d]>>32)|(v[d]<<32);
    v[c]+=v[d];v[b]^=v[c];v[b]=(v[b]>>24)|(v[b]<<40);
    v[a]+=v[b];v[d]^=v[a];v[d]=(v[d]>>16)|(v[d]<<48);
    v[c]+=v[d];v[b]^=v[c];v[b]=(v[b]>>63)|(v[b]<<1); }
  uint64_t x=0;for(unsigned i=0;i<16;i++)x^=v[i];return x;
}''',
    "interp": r'''static uint64_t kernel(uint64_t seed) {
  uint8_t code[192];uint64_t stack[32];for(unsigned i=0;i<192;i++)code[i]=(uint8_t)((i*13+seed)%8);int sp=0;uint64_t acc=0;
  for(unsigned pc=0;pc<192;pc++){uint8_t op=code[pc];switch(op){case 0:stack[sp++]=(uint64_t)pc+seed;break;case 1:if(sp>1){sp--;stack[sp-1]+=stack[sp];}break;case 2:if(sp>1){sp--;stack[sp-1]^=stack[sp];}break;case 3:if(sp)stack[sp-1]*=3;break;case 4:if(sp)stack[sp-1]>>=1;break;case 5:if(sp)acc+=stack[--sp];break;case 6:acc^=pc*0x9e37;break;default:if(sp<30)stack[sp++]=acc;}}
  while(sp)acc+=stack[--sp];return acc;
}''',
    "md5": r'''static uint64_t kernel(uint64_t seed) {
  uint32_t a=0x67452301u,b=0xefcdab89u,c=0x98badcfeu,d=0x10325476u;
  for(unsigned i=0;i<256;i++){uint32_t f=(b&c)|(~b&d),g=i*0x45d9f3bu+seed;uint32_t t=d;d=c;c=b;b+=((a+f+g)<<((i%25)+1))|((a+f+g)>>(32-((i%25)+1)));a=t;}
  return ((uint64_t)a<<32)|b ^ ((uint64_t)c<<17)|d;
}''',
    "sha512": r'''static uint64_t kernel(uint64_t seed) {
  uint64_t h[8];for(unsigned i=0;i<8;i++)h[i]=seed+i*0x6a09e667f3bcc908ull;
  for(unsigned r=0;r<160;r++){uint64_t s0=(h[0]>>1)|(h[0]<<63),s1=(h[4]>>7)|(h[4]<<57),ch=(h[4]&h[5])^(~h[4]&h[6]),maj=(h[0]&h[1])^(h[0]&h[2])^(h[1]&h[2]);uint64_t t1=h[7]+s1+ch+r*0x9e3779b97f4a7c15ull,t2=s0+maj;
    for(int i=7;i>0;i--)h[i]=h[i-1];h[4]+=t1;h[0]=t1+t2; }uint64_t x=0;for(int i=0;i<8;i++)x^=h[i];return x;
}''',
    "keccak": r'''static uint64_t kernel(uint64_t seed) {
  uint64_t a[25];for(unsigned i=0;i<25;i++)a[i]=seed+i*0x9e3779b97f4a7c15ull;
  for(unsigned r=0;r<24;r++){uint64_t c[5],d[5];for(unsigned x=0;x<5;x++)c[x]=a[x]^a[x+5]^a[x+10]^a[x+15]^a[x+20];for(unsigned x=0;x<5;x++)d[x]=c[(x+4)%5]^((c[(x+1)%5]<<1)|(c[(x+1)%5]>>63));
    for(unsigned y=0;y<5;y++)for(unsigned x=0;x<5;x++){unsigned i=x+5*y;uint64_t v=a[i]^d[x];a[i]=(v<<((i*7+r)%63+1))|(v>>(64-((i*7+r)%63+1)));}
    for(unsigned i=0;i<25;i++)a[i]^=(uint64_t)(r+1)*0x100000001b3ull; }uint64_t z=0;for(int i=0;i<25;i++)z^=a[i];return z;
}''',
    "bn_mul": r'''static uint64_t kernel(uint64_t seed) {
  uint64_t a[32],b[32],r[64]={0};for(unsigned i=0;i<32;i++){a[i]=seed+i*0x9e3779b97f4a7c15ull;b[i]=seed^(i*0x100000001b3ull);}
  for(unsigned i=0;i<32;i++){uint64_t carry=0;for(unsigned j=0;j<32;j++){__uint128_t t=(__uint128_t)a[i]*b[j]+r[i+j]+carry;r[i+j]=(uint64_t)t;carry=(uint64_t)(t>>64);}r[i+32]=carry;}
  uint64_t x=0;for(int i=0;i<64;i++)x^=r[i];return x;
}''',
    "tree_walk": r'''struct Node {int left,right;uint64_t value;};
static uint64_t kernel(uint64_t seed) {struct Node n[511];for(int i=0;i<511;i++){n[i].left=2*i+1<511?2*i+1:-1;n[i].right=2*i+2<511?2*i+2:-1;n[i].value=(seed+i*17u)^0x9e3779b9u;}
  int todo[512],sp=0;todo[sp++]=0;uint64_t sum=0;while(sp){int i=todo[--sp];sum=(sum<<5)-sum+n[i].value;if(n[i].right>=0)todo[sp++]=n[i].right;if(n[i].left>=0)todo[sp++]=n[i].left;}return sum;}
''',
    "regex_nfa": r'''static uint64_t kernel(uint64_t seed) {uint64_t active=1,accept=0x80000000ull;for(unsigned i=0;i<512;i++){unsigned char ch=(unsigned char)(seed+i*29);uint64_t cls=(ch&1?0x55555555ull:0x33333333ull)^(ch&2?0x0f0f0f0full:0x00ff00ffull);uint64_t closure=active|(active<<1)|(active<<3)|(active>>2);active=((closure&cls)<<1)|(closure&~cls);active^=(active>>7)&0x1010101ull;if(active&accept)active|=1;}return active;}
''',
    "lz77_match": r'''static uint64_t kernel(uint64_t seed) {uint8_t b[4096];for(unsigned i=0;i<4096;i++)b[i]=(uint8_t)((i*i/7+i*13+seed)%251);uint64_t score=0;for(unsigned p=258;p<4000;p+=3){unsigned best=0,bestoff=0;for(unsigned off=1;off<=64;off++){unsigned len=0;while(len<32&&p+len<4096&&b[p+len]==b[p+len-off])len++;if(len>best){best=len;bestoff=off;}}score=score*33+best*257+bestoff;}return score;}
''',
    "crc32c_3way": r'''static uint64_t kernel(uint64_t seed) {uint32_t a=~(uint32_t)seed,b=a^0x12345678,c=a^0xabcdef01;for(unsigned i=0;i<12000;i++){uint32_t x=(i*0x9e3779b9u)^seed;a=(a>>1)^((0-(a&1))&0x82f63b78u)^x;b=(b>>1)^((0-(b&1))&0x82f63b78u)^(x>>3);c=(c>>1)^((0-(c&1))&0x82f63b78u)^(x<<5);}return ((uint64_t)a<<32)^b^c;}
''',
    "montgomery": r'''static uint64_t kernel(uint64_t seed) {const uint64_t mod=0xffffffffffffffc5ull;uint64_t a=seed|1,b=(seed>>1)|3,x=1;for(unsigned i=0;i<3000;i++){__uint128_t t=(__uint128_t)a*b+x;uint64_t lo=(uint64_t)t,hi=(uint64_t)(t>>64);uint64_t q=lo*0xcccccccccccccccdull;__uint128_t u=(__uint128_t)q*mod;uint64_t v=hi+(uint64_t)(u>>64);x=v>=mod?v-mod:v;a=a*3+1;b=b*5+7;}return x;}
''',
    "matrix_inverse": r'''static uint64_t kernel(uint64_t seed) {double m[4][8];for(int i=0;i<4;i++)for(int j=0;j<8;j++)m[i][j]=j<4?(i==j?4.0+(seed%7):((i+j+seed)%5)*0.1):(j-4==i);
 for(int k=0;k<4;k++){double p=m[k][k];for(int j=0;j<8;j++)m[k][j]/=p;for(int i=0;i<4;i++)if(i!=k){double f=m[i][k];for(int j=0;j<8;j++)m[i][j]-=f*m[k][j];}}uint64_t x=0;for(int i=0;i<4;i++)for(int j=4;j<8;j++)x=x*131+(uint64_t)(m[i][j]*1000000);return x;}
''',
    "radix_histogram": r'''static uint64_t kernel(uint64_t seed) {uint32_t a[2048],hist[256]={0};for(unsigned i=0;i<2048;i++)a[i]=(uint32_t)(seed+i*0x9e3779b9u)^(i>>3);for(unsigned i=0;i<2048;i++)hist[(a[i]>>(i%4*8))&255]++;uint32_t sum=0;for(unsigned i=0;i<256;i++){uint32_t v=hist[i];hist[i]=sum;sum+=v;}uint64_t x=sum;for(unsigned i=0;i<2048;i++)x=x*33+(hist[(a[i]>>(i%4*8))&255]++);return x;}
''',
    "siphash_stream": r'''static uint64_t kernel(uint64_t seed) {uint64_t v0=0x736f6d6570736575ull^seed,v1=0x646f72616e646f6dull,v2=0x6c7967656e657261ull,v3=0x7465646279746573ull;for(unsigned i=0;i<1027;i++){uint64_t m=((uint64_t)i<<32)^seed^i*0x9e3779b97f4a7c15ull;v3^=m;for(int r=0;r<2;r++){v0+=v1;v1=(v1<<13)|(v1>>51);v1^=v0;v0=(v0<<32)|(v0>>32);v2+=v3;v3=(v3<<16)|(v3>>48);v3^=v2;v0+=v3;v3=(v3<<21)|(v3>>43);v3^=v0;v2+=v1;v1=(v1<<17)|(v1>>47);v1^=v2;v2=(v2<<32)|(v2>>32);}v0^=m;}return v0^v1^v2^v3;}
''',
    "geometry_return": r'''struct Vec3 {double x,y,z;};static struct Vec3 transform(struct Vec3 a,struct Vec3 b){struct Vec3 o={a.y*b.z-a.z*b.y+a.x,a.z*b.x-a.x*b.z+a.y,a.x*b.y-a.y*b.x+a.z};return o;}
static uint64_t kernel(uint64_t seed) {struct Vec3 a={seed%17+1,3.25,7.5},b={2.5,seed%29+2,5.75};uint64_t x=0;for(int i=0;i<4000;i++){struct Vec3 c=transform(a,b);x=x*33+(uint64_t)(c.x*1000+c.y*100+c.z*10);a.x+=0.001;a.y-=0.002;b.z+=0.003;}return x;}
''',
    "parser_fragment": r'''static uint64_t parse(const char *s,unsigned *p,unsigned depth){if(depth>24)return 0;uint64_t x=0;while(s[*p]&&s[*p]!=')'){char c=s[(*p)++];if(c>='0'&&c<='9')x=x*10+(c-'0');else if(c=='(')x+=parse(s,p,depth+1);else if(c=='+')x^=0x9e37;else if(c=='*')x=x*3+1;}if(s[*p]==')')(*p)++;return x;}
static uint64_t kernel(uint64_t seed) {char s[4096];unsigned n=0;for(unsigned i=0;i<1000;i++){if(i%17==0)s[n++]='(';s[n++]=(char)('0'+((i+seed)%10));if(i%17==16)s[n++]=')';s[n++]=i%3?'+' : '*';}s[n]=0;unsigned p=0;return parse(s,&p,0);}
''',
}


def render(name: str, body: str) -> str:
    return "/* Generated by tests/ra/gen_rastress.py; do not edit. */\n" \
        "#include <stdint.h>\n#include <stdio.h>\n#include <stddef.h>\n" \
        + body.strip() + "\nint main(void) { printf(\"%llu\\n\", (unsigned long long)kernel(0x123456789abcdef0ull)); return 0; }\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=Path("/tmp/lccc-rastress"))
    parser.add_argument("--list", action="store_true", help="list kernel names and exit")
    args = parser.parse_args()
    if args.list:
        print("\n".join(KERNELS))
        return 0
    args.output_dir.mkdir(parents=True, exist_ok=True)
    for name, body in KERNELS.items():
        (args.output_dir / f"{name}.c").write_text(render(name, body), encoding="utf-8")
    print(f"generated {len(KERNELS)} deterministic RA kernels in {args.output_dir}")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())

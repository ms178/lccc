#define ZSTD_STATIC_LINKING_ONLY
#include "zstd.h"
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <time.h>
#include <errno.h>
#define DECL(p) \
 extern __typeof__(ZSTD_createCCtx) p##_ZSTD_createCCtx; \
 extern __typeof__(ZSTD_freeCCtx) p##_ZSTD_freeCCtx; \
 extern __typeof__(ZSTD_CCtx_setParameter) p##_ZSTD_CCtx_setParameter; \
 extern __typeof__(ZSTD_compress2) p##_ZSTD_compress2;
DECL(base)
DECL(opt)
struct api { ZSTD_CCtx *(*create)(void); size_t (*release)(ZSTD_CCtx*); size_t (*set)(ZSTD_CCtx*,ZSTD_cParameter,int); size_t (*compress)(ZSTD_CCtx*,void*,size_t,const void*,size_t); };
#define API(p) {p##_ZSTD_createCCtx,p##_ZSTD_freeCCtx,p##_ZSTD_CCtx_setParameter,p##_ZSTD_compress2}
static const struct api apis[3] = {API(base),API(opt),API(base)};
static void ck(size_t n) { if(ZSTD_isError(n)){fprintf(stderr,"%s\n",ZSTD_getErrorName(n));exit(2);} }
static uint64_t now(void) {struct timespec t;if(clock_gettime(CLOCK_MONOTONIC,&t))exit(3);return (uint64_t)t.tv_sec*1000000000ULL+t.tv_nsec;}
static unsigned random32(unsigned *s){*s=*s*1664525u+1013904223u;return *s;}
int main(int argc,char **argv) {
 if(argc!=6)return 64;
 int level=atoi(argv[2]),ldm=atoi(argv[3]),rounds=atoi(argv[4]);unsigned seed=(unsigned)strtoul(argv[5],0,10);
 if(rounds<1||rounds>101)return 64;
 FILE*f=fopen(argv[1],"rb");if(!f||fseek(f,0,SEEK_END))return 4;long length=ftell(f);if(length!=1048576)return 4;rewind(f);size_t n=(size_t)length;
 void*src=malloc(n);size_t cap=ZSTD_compressBound(n);void*out=malloc(cap),*plain=malloc(n);if(!src||!out||!plain)return 4;if(fread(src,1,n,f)!=n||fclose(f))return 4;
 ZSTD_CCtx*ctx[3];ZSTD_DCtx*d=ZSTD_createDCtx();if(!d)return 4;
 unsigned creation[3]={0,1,2};for(unsigned i=2;i>0;i--){unsigned j=random32(&seed)%(i+1),v=creation[i];creation[i]=creation[j];creation[j]=v;}
 for(unsigned i=0;i<3;i++){unsigned a=creation[i];ctx[a]=apis[a].create();if(!ctx[a])return 4;ck(apis[a].set(ctx[a],ZSTD_c_compressionLevel,level));ck(apis[a].set(ctx[a],ZSTD_c_checksumFlag,1));if(ldm){ck(apis[a].set(ctx[a],ZSTD_c_enableLongDistanceMatching,1));ck(apis[a].set(ctx[a],ZSTD_c_windowLog,20));}}
 unsigned count=1;
 for(int round=-3;round<rounds;round++) {
  unsigned order[3]={0,1,2};for(unsigned i=2;i>0;i--){unsigned j=random32(&seed)%(i+1),v=order[i];order[i]=order[j];order[j]=v;}
  uint64_t total[3]={0,0,0};size_t sizes[3]={0,0,0};
  for(unsigned slot=0;slot<3;slot++) {
   unsigned a=order[slot];
   /* Explicit equal warm-up after the arm switch; excluded from timing. */
   ck(apis[a].compress(ctx[a],out,cap,src,n));
   for(unsigned j=0;j<count;j++) {
    uint64_t begin=now();size_t got=apis[a].compress(ctx[a],out,cap,src,n);uint64_t end=now();total[a]+=end-begin;ck(got);sizes[a]=got;
    size_t decoded=ZSTD_decompressDCtx(d,plain,n,out,got);ck(decoded);if(decoded!=n||memcmp(src,plain,n))return 5;
   }
  }
  if(round==-3) {uint64_t fastest=total[0];for(unsigned a=1;a<3;a++)if(total[a]<fastest)fastest=total[a];if(!fastest)return 6;uint64_t proposed=(400000000ULL+fastest-1)/fastest;count=(unsigned)(proposed>10000?10000:proposed);if(!count)count=1;}
  else if(round>=0) {printf("%d %u %u %u %u %llu %llu %llu %zu %zu %zu\n",round,order[0],order[1],order[2],count,(unsigned long long)total[0],(unsigned long long)total[1],(unsigned long long)total[2],sizes[0],sizes[1],sizes[2]);fflush(stdout);}
 }
 for(unsigned a=0;a<3;a++)ck(apis[a].release(ctx[a]));
 ck(ZSTD_freeDCtx(d));free(src);free(out);free(plain);return 0;
}

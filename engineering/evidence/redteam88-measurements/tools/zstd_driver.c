#define ZSTD_STATIC_LINKING_ONLY
#include "zstd.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <stdint.h>
static void ck(size_t n) { if (ZSTD_isError(n)) { fprintf(stderr,"%s\n", ZSTD_getErrorName(n)); exit(2); } }
static uint64_t ns(void) { struct timespec t; if(clock_gettime(CLOCK_MONOTONIC,&t)) exit(3); return (uint64_t)t.tv_sec*1000000000ULL+t.tv_nsec; }
static void *readfile(const char *p, size_t *n) { FILE *f=fopen(p,"rb"); if(!f) exit(4); if(fseek(f,0,SEEK_END))exit(4);long z=ftell(f);if(z<0)exit(4);rewind(f);*n=(size_t)z;void *b=malloc(*n+1);if(!b||fread(b,1,*n,f)!=*n)exit(4);fclose(f);return b; }
int main(int argc,char **argv) {
 if(argc!=10)return 64;
 size_t n, encoded_n=0; void *src=readfile(argv[1],&n);
 int level=atoi(argv[2]), workers=atoi(argv[3]), ldm=atoi(argv[4]), checksum=atoi(argv[5]), count=atoi(argv[6]);
 int decode=atoi(argv[7]); if(count<1||count>10000)return 64;
 size_t cap=ZSTD_compressBound(n), csize=0; void *compressed=malloc(cap),*plain=malloc(n+1),*encoded=0;
 if(!compressed||!plain)return 4;
 ZSTD_CCtx *c=ZSTD_createCCtx();ZSTD_DCtx*d=ZSTD_createDCtx();if(!c||!d)return 4;
 ck(ZSTD_CCtx_setParameter(c,ZSTD_c_compressionLevel,level));
 ck(ZSTD_CCtx_setParameter(c,ZSTD_c_nbWorkers,workers));
 ck(ZSTD_CCtx_setParameter(c,ZSTD_c_checksumFlag,checksum));
 if(ldm){ck(ZSTD_CCtx_setParameter(c,ZSTD_c_enableLongDistanceMatching,1));ck(ZSTD_CCtx_setParameter(c,ZSTD_c_windowLog,20));}
 if(decode)encoded=readfile(argv[8],&encoded_n);
 uint64_t elapsed=0;
 for(int k=0;k<count;k++) {
  uint64_t begin=ns();
  size_t got=decode ? ZSTD_decompressDCtx(d,plain,n,encoded,encoded_n) : ZSTD_compress2(c,compressed,cap,src,n);
  elapsed+=ns()-begin;ck(got);
  if(!decode){csize=got;got=ZSTD_decompressDCtx(d,plain,n,compressed,csize);ck(got);}
  if(got!=n||memcmp(src,plain,n)){fprintf(stderr,"roundtrip mismatch\n");return 5;}
 }
 if(!decode){FILE*f=fopen(argv[9],"wb");if(!f||fwrite(compressed,1,csize,f)!=csize||fclose(f))return 6;}
 printf("%llu %d %zu %zu\n",(unsigned long long)elapsed,count,n,decode?encoded_n:csize);
 ZSTD_freeCCtx(c);ZSTD_freeDCtx(d);free(src);free(compressed);free(plain);free(encoded);return 0;
}

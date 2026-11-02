#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <lz4frame.h>
size_t LZ4F_compressBegin(LZ4F_cctx *ctx,void *dst,size_t cap,const LZ4F_preferences_t *p) {
 size_t (*next)(LZ4F_cctx*,void*,size_t,const LZ4F_preferences_t*)=dlsym(RTLD_NEXT,"LZ4F_compressBegin");
 fprintf(stderr,"prefs bytes=%zu block=%d mode=%d checksum=%d type=%d content=%llu dict=%u blockcheck=%d level=%d auto=%u favor=%u reserved=%u,%u,%u cap=%zu\n",sizeof(*p),p->frameInfo.blockSizeID,p->frameInfo.blockMode,p->frameInfo.contentChecksumFlag,p->frameInfo.frameType,p->frameInfo.contentSize,p->frameInfo.dictID,p->frameInfo.blockChecksumFlag,p->compressionLevel,p->autoFlush,p->favorDecSpeed,p->reserved[0],p->reserved[1],p->reserved[2],cap);
 return next(ctx,dst,cap,p);
}

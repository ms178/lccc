#include <stdint.h>
#include <stdio.h>
typedef unsigned d4 __attribute__((vector_size(16)));
extern d4 lccc_d4sl(d4,unsigned);
extern d4 gcc_d4sl(d4,unsigned);
extern d4 clang_d4sl(d4,unsigned);
extern d4 lccc_d4sr(d4,unsigned);
extern d4 gcc_d4sr(d4,unsigned);
extern d4 clang_d4sr(d4,unsigned);
extern d4 lccc_d4sa(d4,unsigned);
extern d4 gcc_d4sa(d4,unsigned);
extern d4 clang_d4sa(d4,unsigned);
typedef unsigned d8 __attribute__((vector_size(32)));
extern d8 lccc_d8sl(d8,unsigned);
extern d8 gcc_d8sl(d8,unsigned);
extern d8 clang_d8sl(d8,unsigned);
extern d8 lccc_d8sr(d8,unsigned);
extern d8 gcc_d8sr(d8,unsigned);
extern d8 clang_d8sr(d8,unsigned);
extern d8 lccc_d8sa(d8,unsigned);
extern d8 gcc_d8sa(d8,unsigned);
extern d8 clang_d8sa(d8,unsigned);
typedef unsigned long long q2 __attribute__((vector_size(16)));
extern q2 lccc_q2sl(q2,unsigned);
extern q2 gcc_q2sl(q2,unsigned);
extern q2 clang_q2sl(q2,unsigned);
extern q2 lccc_q2sr(q2,unsigned);
extern q2 gcc_q2sr(q2,unsigned);
extern q2 clang_q2sr(q2,unsigned);
typedef unsigned long long q4 __attribute__((vector_size(32)));
extern q4 lccc_q4sl(q4,unsigned);
extern q4 gcc_q4sl(q4,unsigned);
extern q4 clang_q4sl(q4,unsigned);
extern q4 lccc_q4sr(q4,unsigned);
extern q4 gcc_q4sr(q4,unsigned);
extern q4 clang_q4sr(q4,unsigned);
int main(void){uint64_t state=0xbadbeef0155ULL;unsigned calls=0;for(unsigned n=0;n<32;n++) for(unsigned k=0;k<1000;k++){
 d4 a; for(unsigned j=0;j<4;j++){state^=state<<13;state^=state>>7;state^=state<<17;a[j]=state;}
 d4 x=lccc_d4sl(a,n),y=gcc_d4sl(a,n),z=clang_d4sl(a,n);
 for(unsigned j=0;j<4;j++) if(x[j]!=y[j]||x[j]!=z[j]){fprintf(stderr,"d4sl n=%u lane=%u\n",n,j);return 1;} calls++;
 }
for(unsigned n=0;n<32;n++) for(unsigned k=0;k<1000;k++){
 d4 a; for(unsigned j=0;j<4;j++){state^=state<<13;state^=state>>7;state^=state<<17;a[j]=state;}
 d4 x=lccc_d4sr(a,n),y=gcc_d4sr(a,n),z=clang_d4sr(a,n);
 for(unsigned j=0;j<4;j++) if(x[j]!=y[j]||x[j]!=z[j]){fprintf(stderr,"d4sr n=%u lane=%u\n",n,j);return 1;} calls++;
 }
for(unsigned n=0;n<32;n++) for(unsigned k=0;k<1000;k++){
 d4 a; for(unsigned j=0;j<4;j++){state^=state<<13;state^=state>>7;state^=state<<17;a[j]=state;}
 d4 x=lccc_d4sa(a,n),y=gcc_d4sa(a,n),z=clang_d4sa(a,n);
 for(unsigned j=0;j<4;j++) if(x[j]!=y[j]||x[j]!=z[j]){fprintf(stderr,"d4sa n=%u lane=%u\n",n,j);return 1;} calls++;
 }
for(unsigned n=0;n<32;n++) for(unsigned k=0;k<1000;k++){
 d8 a; for(unsigned j=0;j<8;j++){state^=state<<13;state^=state>>7;state^=state<<17;a[j]=state;}
 d8 x=lccc_d8sl(a,n),y=gcc_d8sl(a,n),z=clang_d8sl(a,n);
 for(unsigned j=0;j<8;j++) if(x[j]!=y[j]||x[j]!=z[j]){fprintf(stderr,"d8sl n=%u lane=%u\n",n,j);return 1;} calls++;
 }
for(unsigned n=0;n<32;n++) for(unsigned k=0;k<1000;k++){
 d8 a; for(unsigned j=0;j<8;j++){state^=state<<13;state^=state>>7;state^=state<<17;a[j]=state;}
 d8 x=lccc_d8sr(a,n),y=gcc_d8sr(a,n),z=clang_d8sr(a,n);
 for(unsigned j=0;j<8;j++) if(x[j]!=y[j]||x[j]!=z[j]){fprintf(stderr,"d8sr n=%u lane=%u\n",n,j);return 1;} calls++;
 }
for(unsigned n=0;n<32;n++) for(unsigned k=0;k<1000;k++){
 d8 a; for(unsigned j=0;j<8;j++){state^=state<<13;state^=state>>7;state^=state<<17;a[j]=state;}
 d8 x=lccc_d8sa(a,n),y=gcc_d8sa(a,n),z=clang_d8sa(a,n);
 for(unsigned j=0;j<8;j++) if(x[j]!=y[j]||x[j]!=z[j]){fprintf(stderr,"d8sa n=%u lane=%u\n",n,j);return 1;} calls++;
 }
for(unsigned n=0;n<64;n++) for(unsigned k=0;k<1000;k++){
 q2 a; for(unsigned j=0;j<2;j++){state^=state<<13;state^=state>>7;state^=state<<17;a[j]=state;}
 q2 x=lccc_q2sl(a,n),y=gcc_q2sl(a,n),z=clang_q2sl(a,n);
 for(unsigned j=0;j<2;j++) if(x[j]!=y[j]||x[j]!=z[j]){fprintf(stderr,"q2sl n=%u lane=%u\n",n,j);return 1;} calls++;
 }
for(unsigned n=0;n<64;n++) for(unsigned k=0;k<1000;k++){
 q2 a; for(unsigned j=0;j<2;j++){state^=state<<13;state^=state>>7;state^=state<<17;a[j]=state;}
 q2 x=lccc_q2sr(a,n),y=gcc_q2sr(a,n),z=clang_q2sr(a,n);
 for(unsigned j=0;j<2;j++) if(x[j]!=y[j]||x[j]!=z[j]){fprintf(stderr,"q2sr n=%u lane=%u\n",n,j);return 1;} calls++;
 }
for(unsigned n=0;n<64;n++) for(unsigned k=0;k<1000;k++){
 q4 a; for(unsigned j=0;j<4;j++){state^=state<<13;state^=state>>7;state^=state<<17;a[j]=state;}
 q4 x=lccc_q4sl(a,n),y=gcc_q4sl(a,n),z=clang_q4sl(a,n);
 for(unsigned j=0;j<4;j++) if(x[j]!=y[j]||x[j]!=z[j]){fprintf(stderr,"q4sl n=%u lane=%u\n",n,j);return 1;} calls++;
 }
for(unsigned n=0;n<64;n++) for(unsigned k=0;k<1000;k++){
 q4 a; for(unsigned j=0;j<4;j++){state^=state<<13;state^=state>>7;state^=state<<17;a[j]=state;}
 q4 x=lccc_q4sr(a,n),y=gcc_q4sr(a,n),z=clang_q4sr(a,n);
 for(unsigned j=0;j<4;j++) if(x[j]!=y[j]||x[j]!=z[j]){fprintf(stderr,"q4sr n=%u lane=%u\n",n,j);return 1;} calls++;
 }
printf("%u compiler differential calls\n",calls);return 0;}

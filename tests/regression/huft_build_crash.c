#include <stdio.h>
#include <stdlib.h>
typedef unsigned char uch;
typedef unsigned short ush;
#define BMAX 15
#define N_MAX 288
struct huft { uch e; uch b; union { ush n; struct huft *t; } v; };
int hufts=0;
static void memzero(void *s, unsigned n){ unsigned char *p=s; while(n--) *p++=0; }
static int huft_free(struct huft *t){ (void)t; return 0; }
#include "huft_build_body.inc"
int main(void){
  unsigned b[19] = {4,0,0,7,4,4,4,2,3,3,4,4,4,0,0,0,5,7,6};
  struct huft *tl = 0; int bl = 7;
  int rc = huft_build(b, 19, 19, 0, 0, &tl, &bl);
  fprintf(stderr, "rc=%d bl=%d\n", rc, bl);
  return rc==0 ? 0 : 1;
}

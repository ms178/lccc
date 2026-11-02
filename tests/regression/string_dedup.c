/* String literal dedup: identical literals must share address (GCC -fmerge-constants). */
#include <stdio.h>
int main(void){ char *a="merge"; char *b="merge";
  printf("%d\n", a==b); return (a==b)?0:1; }

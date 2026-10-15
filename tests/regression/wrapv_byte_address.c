/* The narrow signed product/addend must wrap BEFORE pointer extension.
   This is a byte-address shape, unlike the older int-element domain probe. */
__attribute__((noinline)) int scale4(const char *p, int start, int end) {
    int sum=0; for(int i=start;i<end;i++) sum+=p[i*4]; return sum;
}
__attribute__((noinline)) int scale2(const char *p, int start, int end) {
    int sum=0; for(int i=start;i<end;i++) sum+=p[i*2]; return sum;
}
__attribute__((noinline)) int scale8(const char *p, int start, int end) {
    int sum=0; for(int i=start;i<end;i++) sum+=p[i*8]; return sum;
}
__attribute__((noinline)) int plus3(const char *p, int start, int end) {
    int sum=0; for(int i=start;i<end;i++) sum+=p[i*4+3]; return sum;
}
__attribute__((noinline)) int minus3(const char *p, int start, int end) {
    int sum=0; for(int i=start;i<end;i++) sum+=p[i*4-3]; return sum;
}
__attribute__((noinline)) void store4(char *p, int start, int end) {
    for(int i=start;i<end;i++) p[i*4]=(char)(i-start+1);
}
__attribute__((noinline)) int ptr_left(const char *p, int start, int end) {
    int sum=0; for(int i=start;i<end;i++) sum+=*(p+(i*4-3)); return sum;
}
__attribute__((noinline)) int ptr_right(const char *p, int start, int end) {
    int sum=0; for(int i=start;i<end;i++) sum+=*((i*4-3)+p); return sum;
}
__attribute__((noinline)) int ptr_sub(const char *p, int start, int end) {
    int sum=0; for(int i=start;i<end;i++) sum+=*(p-(3-i*4)); return sum;
}
__attribute__((noinline)) int ptr_compound(const char *p, int start, int end) {
    int sum=0; for(int i=start;i<end;i++){const char *q=p;q+=(i*4-3);sum+=*q;} return sum;
}
int main(void) {
    char p[64]; for(int i=0;i<64;i++)p[i]=(char)(i+1);
    if(scale4(p,1073741824,1073741828)!=28)return 1;
    if(scale2(p,-2147483647-1,-2147483644)!=16)return 2;
    if(scale8(p,536870912,536870916)!=52)return 3;
    if(plus3(p,1073741824,1073741828)!=40)return 4;
    if(minus3(p+3,1073741824,1073741828)!=28)return 5;
    if(ptr_left(p+3,1073741824,1073741828)!=28)return 7;
    if(ptr_right(p+3,1073741824,1073741828)!=28)return 8;
    if(ptr_sub(p+3,1073741824,1073741828)!=28)return 9;
    if(ptr_compound(p+3,1073741824,1073741828)!=28)return 10;
    // Negative index without any overflow: valid in the default contract too.
    if(minus3(p+3,0,4)!=28 || ptr_left(p+3,0,4)!=28)return 11;
    store4(p,1073741824,1073741828);
    if(p[0]!=1 || p[4]!=2 || p[8]!=3 || p[12]!=4)return 6;
    return 0;
}

extern int check(const unsigned short *, const unsigned char *, const unsigned short *);
static unsigned char guard[256];
static unsigned short tree[288];
int main(void) {
    unsigned short count[16];
    int n;
    for (n=0;n<256;n++) guard[n]=(unsigned char)n;
    goto init;
loop:
    n=0;
    while (n<144) { tree[n++]=8; count[8]++; }
    return check(count,guard,tree);
init:
    for (n=0;n<16;n++) count[n]=0;
    goto loop;
}

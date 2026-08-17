struct S { int a,b,c,d,e,f,g,h; };
int main(void) {
    struct S s = {1,2,3,4,5,6,7,8};
    return s.a+s.h == 9 ? 0 : 1;
}

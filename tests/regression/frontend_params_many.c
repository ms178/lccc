static int f(int a,int b,int c,int d,int e,int f,int g,int h) {
    return a+b+c+d+e+f+g+h;
}
int main(void) { return f(1,1,1,1,1,1,1,1) == 8 ? 0 : 1; }

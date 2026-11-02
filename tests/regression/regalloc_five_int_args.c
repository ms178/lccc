
int add5(int a, int b, int c, int d, int e) {
    return a + b + c + d + e;
}
int main(void) {
    if (add5(1,2,3,4,5) != 15) return 1;
    if (add5(-1,-2,-3,-4,-5) != -15) return 2;
    return 0;
}

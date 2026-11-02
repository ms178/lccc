int main(void) {
    int a[4] = {3,1,4,1};
    int *p = a;
    int s = 0;
    for (int i = 0; i < 4; i++) { s += *p; p++; }
    return s == 9 ? 0 : 1;
}

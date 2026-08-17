
/* Multiple pointer + integer args must not clobber each other via ParamRef homes */
long long mix(long long *a, long long *b, long long *c, int n, int k) {
    long long s = 0;
    for (int i = 0; i < n; i++)
        s += a[i] * b[i] + c[i] * k;
    return s;
}
int main(void) {
    long long a[4] = {1,2,3,4}, b[4] = {5,6,7,8}, c[4] = {9,10,11,12};
    long long s = mix(a, b, c, 4, 3);
    long long expect = 0;
    for (int i = 0; i < 4; i++) expect += a[i]*b[i] + c[i]*3;
    return s == expect ? 0 : 1;
}

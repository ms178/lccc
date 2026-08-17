int main(void) {
    int a = 0, b = 1;
    for (int i = 0; i < 4; i++) { a += i; b *= 2; }
    return (a == 6 && b == 16) ? 0 : 1;
}

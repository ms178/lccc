int main(void) {
    int s = 0;
    for (int i = 0; i < 4; i++) s += i + 1;
    return s == 10 ? 0 : 1;
}

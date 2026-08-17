typedef struct { int x, y; } P;
int main(void) {
    P g[4];
    for (int i = 0; i < 4; i++) { g[i].x = i + 1; g[i].y = (i + 1) * 10; }
    int s = 0;
    for (int i = 0; i < 4; i++) s += g[i].x + g[i].y;
    return s == 110 ? 0 : 1;
}

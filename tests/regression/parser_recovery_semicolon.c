/* Valid program: recovery path must not break normal parses. */
int main(void) {
    int a = 1;
    int b = 2;
    return a + b - 3;
}

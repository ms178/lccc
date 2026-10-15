unsigned count;
__attribute__((noinline)) unsigned f(void) {
  count=0;
  for (int i=-100; i+16<0; i+=(-2147483550)) ++count;
  return count;
}
int main(void) { return f()!=2; }

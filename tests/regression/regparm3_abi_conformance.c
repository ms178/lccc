int fails;
#define T(n, got, want) do { if ((long long)(got) != (long long)(want)) fails |= (1<<n); } while(0)
int __attribute__((noinline)) i3(int a, int b, int c) { return a*100+b*10+c; }
long long __attribute__((noinline)) ll1(long long a) { return a+1; }
long long __attribute__((noinline)) i_ll(int a, long long b) { return a+b; }
int __attribute__((noinline)) u16_u8(unsigned short a, unsigned char b) { return a+b; }
struct S8 { int a, b; };
int __attribute__((noinline)) st_i(struct S8 s, int b) { return s.a*10 + s.b + b; }
double __attribute__((noinline)) d_i(double a, int b) { return a+b; }
int __attribute__((noinline)) vsum(int n, ...) {
  __builtin_va_list ap; __builtin_va_start(ap, n);
  int s = 0; for (int i=0;i<n;i++) s += __builtin_va_arg(ap, int);
  __builtin_va_end(ap); return s;
}
int (* volatile pi3)(int,int,int) = i3;
struct S8 __attribute__((noinline)) mk(int a, int b) { struct S8 r = {a+1, b+2}; return r; }
int main(void) {
  T(0, i3(1,2,3), 123);
  T(1, ll1(0x123456789ALL), 0x123456789BLL);
  T(2, i_ll(7, 0x200000000LL), 0x200000007LL);
  T(3, u16_u8(60000, 200), 60200);
  struct S8 s = {3, 4};
  T(4, st_i(s, 5), 39);
  T(5, (int)(d_i(1.5,2)*2), 7);
  T(6, vsum(3, 10, 20, 30), 60);
  T(7, pi3(4,5,6), 456);
  struct S8 m = mk(1,2);
  T(8, m.a*10+m.b, 24);
  return fails;
}

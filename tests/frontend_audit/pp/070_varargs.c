#define V(...) f(__VA_ARGS__)
#define VC(fmt, ...) g(fmt, ##__VA_ARGS__)
#define VN(fmt, args...) h(fmt, ##args)
#define VO(fmt, ...) v(fmt __VA_OPT__(,) __VA_ARGS__)
V();
V(1);
V(1,2,3);
VC("x");
VC("x", 1);
VC("x", 1, 2);
VN("x");
VN("x", 1);
VO("x");
VO("x", 1);
VO("x", 1, 2);
#define VS(...) #__VA_ARGS__
VS();
VS(1, 2);
VS(a b );
#define PAREN(...) (__VA_ARGS__)
PAREN();

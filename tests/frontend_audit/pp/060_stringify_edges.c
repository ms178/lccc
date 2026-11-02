#define S(x) #x
#define XS(x) S(x)
S();
S( );
S(a   b);
S("a\nb");
S('\\');
S(a /* comment */ b);
S(a//comment
b);
S(x
  y);
S(#);
S(##);
#define EMPTY
S(EMPTY);
XS(EMPTY);
S(1.0e+5);
S(@);
S($);
S(\u00e9);
S(__VA_ARGS__);

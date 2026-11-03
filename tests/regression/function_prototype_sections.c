/* Linux __init-shaped declarations must govern an unannotated definition,
 * including a weak alias with the same prior annotation. These attributes
 * belong to function entities, not function-pointer objects. */
int early(int) __attribute__((section(".init.text")));
int early(int x) { return x + 1; }
extern int early_alias(int)
    __attribute__((weak, alias("early"), section(".init.text")));

typedef int function_type(int);
extern function_type typed __attribute__((section(".typed.text")));
int typed(int x) { return x + 2; }

static int local(int) __attribute__((section(".local.text")));
__attribute__((used)) static int local(int x) { return x + 3; }

int cold_function(int) __attribute__((cold, section(".init.text")));
__attribute__((cold)) int cold_function(int x) { return x + 4; }

int repeated(int) __attribute__((section(".repeated.text")));
int repeated(int) __attribute__((section(".repeated.text")));
int repeated(int x) { return x + 5; }

int plain(int x) { return x + 6; }
int (*pointer_object)(int) __attribute__((section(".pointer.data"))) = plain;

int main(void) {
    if (early(10) != 11 || early_alias(20) != 21) return 1;
    if (typed(30) != 32 || local(40) != 43) return 2;
    if (cold_function(50) != 54 || repeated(60) != 65) return 3;
    if (pointer_object(70) != 76) return 4;
    return 0;
}

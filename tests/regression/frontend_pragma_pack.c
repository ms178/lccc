#pragma pack(push, 1)
struct T { char c; int i; };
#pragma pack(pop)
int main(void) { return sizeof(struct T) >= 5 ? 0 : 1; }

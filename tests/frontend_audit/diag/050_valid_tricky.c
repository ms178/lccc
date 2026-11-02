/* expect: accept */
typedef struct node { struct node *next; int value; } node;
enum { A = sizeof(node), B = A * 2 };
static int f(int n) { return n ? f(n - 1) + 1 : 0; }
int main(void) {
    node n = { 0, B };
    int x = (int){ 1 } + f(3);
    return x + n.value - B;
}

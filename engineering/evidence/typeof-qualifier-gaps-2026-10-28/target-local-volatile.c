void local_volatile(void) { volatile int x=7; x; x; }
int param_volatile(volatile int p) { return p+p; }
volatile int global;
void global_volatile(void) { global; global; }

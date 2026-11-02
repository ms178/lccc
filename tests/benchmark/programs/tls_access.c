__thread unsigned long tls_counter;
extern __thread unsigned long external_counter __attribute__((weak));

unsigned long tls_load(void) { return tls_counter; }
void tls_store(unsigned long value) { tls_counter = value; }
unsigned long *tls_address(void) { return &tls_counter; }
unsigned long external_load(void) { return external_counter; }
unsigned long *external_address(void) { return &external_counter; }

int main(void) {
    tls_store(7);
    return tls_load() != 7;
}

extern unsigned long *external_address(void);
extern unsigned long *local_address(void);
extern unsigned long external_load(void);
extern void external_store(unsigned long);

int main(void) {
    unsigned long *external = external_address();
    unsigned long *local = local_address();
    external_store(0x1122334455667788UL);
    *local = 0xaabbccddeeff0011UL;
    return external != external_address() || local != local_address() ||
           external_load() != 0x1122334455667788UL ||
           *external != 0x1122334455667788UL ||
           *local != 0xaabbccddeeff0011UL;
}

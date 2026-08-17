int main(void) {
    unsigned crc = 0xffffffffu;
    unsigned char buf[4] = {1,2,3,4};
    for (int i = 0; i < 4; i++) {
        crc ^= buf[i];
        for (int k = 0; k < 8; k++)
            crc = (crc >> 1) ^ (0x82F63B78u & -(crc & 1u));
    }
    unsigned ref = 0xffffffffu;
    for (int i = 0; i < 4; i++) {
        ref ^= buf[i];
        for (int k = 0; k < 8; k++)
            ref = (ref >> 1) ^ (0x82F63B78u & -(ref & 1u));
    }
    return crc == ref ? 0 : 1;
}

typedef ELEM T __attribute__((ext_vector_type(LANES)));
T getx(void);
T sinkr_v __asm__("sinkr");
const unsigned sink_size = sizeof(T), lane_size = LSIZE;
void call_get(void) { sinkr_v = getx(); }

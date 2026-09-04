// Regression: post-phi copy cleanup must resolve copy chains longer than
// 32 links.  After every check block below folds, the `fails` accumulator
// is a chain of 48 `Copy` instructions; propagate_copies_post_phi resolved
// chains with a hard `depth < 32` cap, so the survivor was rewritten to an
// intermediate value whose own copy had been removed, and the backend's
// no-home hard gate fired (-O1 ICE: "operand_to_rax: value has no register
// home").  Found by tests/stress/run_stress.py (divmod/shifts/builtins).
// Unit test: passes::copy_prop::tests::test_post_phi_long_chain_no_dangling_use
#include <stdio.h>
#include <stdint.h>

static inline uint32_t k0(uint32_t p) { return (p / 3u) ^ (p % 3u); }
static inline uint32_t k1(uint32_t p) { return (p / 4u) ^ (p % 4u); }
static inline uint32_t k2(uint32_t p) { return (p / 5u) ^ (p % 5u); }
static inline uint32_t k3(uint32_t p) { return (p / 6u) ^ (p % 6u); }
static inline uint32_t k4(uint32_t p) { return (p / 7u) ^ (p % 7u); }
static inline uint32_t k5(uint32_t p) { return (p / 8u) ^ (p % 8u); }
static inline uint32_t k6(uint32_t p) { return (p / 9u) ^ (p % 9u); }
static inline uint32_t k7(uint32_t p) { return (p / 10u) ^ (p % 10u); }
static inline uint32_t k8(uint32_t p) { return (p / 11u) ^ (p % 11u); }
static inline uint32_t k9(uint32_t p) { return (p / 12u) ^ (p % 12u); }
static inline uint32_t k10(uint32_t p) { return (p / 13u) ^ (p % 13u); }
static inline uint32_t k11(uint32_t p) { return (p / 14u) ^ (p % 14u); }
static inline uint32_t k12(uint32_t p) { return (p / 15u) ^ (p % 15u); }
static inline uint32_t k13(uint32_t p) { return (p / 16u) ^ (p % 16u); }
static inline uint32_t k14(uint32_t p) { return (p / 17u) ^ (p % 17u); }
static inline uint32_t k15(uint32_t p) { return (p / 18u) ^ (p % 18u); }
static inline uint32_t k16(uint32_t p) { return (p / 19u) ^ (p % 19u); }
static inline uint32_t k17(uint32_t p) { return (p / 20u) ^ (p % 20u); }
static inline uint32_t k18(uint32_t p) { return (p / 21u) ^ (p % 21u); }
static inline uint32_t k19(uint32_t p) { return (p / 22u) ^ (p % 22u); }
static inline uint32_t k20(uint32_t p) { return (p / 23u) ^ (p % 23u); }
static inline uint32_t k21(uint32_t p) { return (p / 24u) ^ (p % 24u); }
static inline uint32_t k22(uint32_t p) { return (p / 25u) ^ (p % 25u); }
static inline uint32_t k23(uint32_t p) { return (p / 26u) ^ (p % 26u); }
static inline uint32_t k24(uint32_t p) { return (p / 27u) ^ (p % 27u); }
static inline uint32_t k25(uint32_t p) { return (p / 28u) ^ (p % 28u); }
static inline uint32_t k26(uint32_t p) { return (p / 29u) ^ (p % 29u); }
static inline uint32_t k27(uint32_t p) { return (p / 30u) ^ (p % 30u); }
static inline uint32_t k28(uint32_t p) { return (p / 31u) ^ (p % 31u); }
static inline uint32_t k29(uint32_t p) { return (p / 32u) ^ (p % 32u); }
static inline uint32_t k30(uint32_t p) { return (p / 33u) ^ (p % 33u); }
static inline uint32_t k31(uint32_t p) { return (p / 34u) ^ (p % 34u); }
static inline uint32_t k32(uint32_t p) { return (p / 35u) ^ (p % 35u); }
static inline uint32_t k33(uint32_t p) { return (p / 36u) ^ (p % 36u); }
static inline uint32_t k34(uint32_t p) { return (p / 37u) ^ (p % 37u); }
static inline uint32_t k35(uint32_t p) { return (p / 38u) ^ (p % 38u); }
static inline uint32_t k36(uint32_t p) { return (p / 39u) ^ (p % 39u); }
static inline uint32_t k37(uint32_t p) { return (p / 40u) ^ (p % 40u); }
static inline uint32_t k38(uint32_t p) { return (p / 41u) ^ (p % 41u); }
static inline uint32_t k39(uint32_t p) { return (p / 42u) ^ (p % 42u); }
static inline uint32_t k40(uint32_t p) { return (p / 43u) ^ (p % 43u); }
static inline uint32_t k41(uint32_t p) { return (p / 44u) ^ (p % 44u); }
static inline uint32_t k42(uint32_t p) { return (p / 45u) ^ (p % 45u); }
static inline uint32_t k43(uint32_t p) { return (p / 46u) ^ (p % 46u); }
static inline uint32_t k44(uint32_t p) { return (p / 47u) ^ (p % 47u); }
static inline uint32_t k45(uint32_t p) { return (p / 48u) ^ (p % 48u); }
static inline uint32_t k46(uint32_t p) { return (p / 49u) ^ (p % 49u); }
static inline uint32_t k47(uint32_t p) { return (p / 50u) ^ (p % 50u); }

int main(void) {
    int fails = 0;
    { uint32_t r = k0(0u); if (r != 0u) { fails++; printf("FAIL k0 got %u\n", r); } }
    { uint32_t r = k1(2654435761u); if (r != 663608941u) { fails++; printf("FAIL k1 got %u\n", r); } }
    { uint32_t r = k2(1013904226u); if (r != 202780844u) { fails++; printf("FAIL k2 got %u\n", r); } }
    { uint32_t r = k3(3668339987u); if (r != 611389992u) { fails++; printf("FAIL k3 got %u\n", r); } }
    { uint32_t r = k4(2027808452u); if (r != 289686924u) { fails++; printf("FAIL k4 got %u\n", r); } }
    { uint32_t r = k5(387276917u); if (r != 48409611u) { fails++; printf("FAIL k5 got %u\n", r); } }
    { uint32_t r = k6(3041712678u); if (r != 337968072u) { fails++; printf("FAIL k6 got %u\n", r); } }
    { uint32_t r = k7(1401181143u); if (r != 140118113u) { fails++; printf("FAIL k7 got %u\n", r); } }
    { uint32_t r = k8(4055616904u); if (r != 368692436u) { fails++; printf("FAIL k8 got %u\n", r); } }
    { uint32_t r = k9(2415085369u); if (r != 201257115u) { fails++; printf("FAIL k9 got %u\n", r); } }
    { uint32_t r = k10(774553834u); if (r != 59581066u) { fails++; printf("FAIL k10 got %u\n", r); } }
    { uint32_t r = k11(3428989595u); if (r != 244927831u) { fails++; printf("FAIL k11 got %u\n", r); } }
    { uint32_t r = k12(1788458060u); if (r != 119230540u) { fails++; printf("FAIL k12 got %u\n", r); } }
    { uint32_t r = k13(147926525u); if (r != 9245394u) { fails++; printf("FAIL k13 got %u\n", r); } }
    { uint32_t r = k14(2802362286u); if (r != 164844846u) { fails++; printf("FAIL k14 got %u\n", r); } }
    { uint32_t r = k15(1161830751u); if (r != 64546151u) { fails++; printf("FAIL k15 got %u\n", r); } }
    { uint32_t r = k16(3816266512u); if (r != 200856128u) { fails++; printf("FAIL k16 got %u\n", r); } }
    { uint32_t r = k17(2175734977u); if (r != 108786733u) { fails++; printf("FAIL k17 got %u\n", r); } }
    { uint32_t r = k18(535203442u); if (r != 25485874u) { fails++; printf("FAIL k18 got %u\n", r); } }
    { uint32_t r = k19(3189639203u); if (r != 144983603u) { fails++; printf("FAIL k19 got %u\n", r); } }
    { uint32_t r = k20(1549107668u); if (r != 67352508u) { fails++; printf("FAIL k20 got %u\n", r); } }
    { uint32_t r = k21(4203543429u); if (r != 175147631u) { fails++; printf("FAIL k21 got %u\n", r); } }
    { uint32_t r = k22(2563011894u); if (r != 102520456u) { fails++; printf("FAIL k22 got %u\n", r); } }
    { uint32_t r = k23(922480359u); if (r != 35480024u) { fails++; printf("FAIL k23 got %u\n", r); } }
    { uint32_t r = k24(3576916120u); if (r != 132478384u) { fails++; printf("FAIL k24 got %u\n", r); } }
    { uint32_t r = k25(1936384585u); if (r != 69156601u) { fails++; printf("FAIL k25 got %u\n", r); } }
    { uint32_t r = k26(295853050u); if (r != 10201836u) { fails++; printf("FAIL k26 got %u\n", r); } }
    { uint32_t r = k27(2950288811u); if (r != 98342971u) { fails++; printf("FAIL k27 got %u\n", r); } }
    { uint32_t r = k28(1309757276u); if (r != 42250220u) { fails++; printf("FAIL k28 got %u\n", r); } }
    { uint32_t r = k29(3964193037u); if (r != 123881029u) { fails++; printf("FAIL k29 got %u\n", r); } }
    { uint32_t r = k30(2323661502u); if (r != 70414014u) { fails++; printf("FAIL k30 got %u\n", r); } }
    { uint32_t r = k31(683129967u); if (r != 20092036u) { fails++; printf("FAIL k31 got %u\n", r); } }
    { uint32_t r = k32(3337565728u); if (r != 95359024u) { fails++; printf("FAIL k32 got %u\n", r); } }
    { uint32_t r = k33(1697034193u); if (r != 47139815u) { fails++; printf("FAIL k33 got %u\n", r); } }
    { uint32_t r = k34(56502658u); if (r != 1527066u) { fails++; printf("FAIL k34 got %u\n", r); } }
    { uint32_t r = k35(2710938419u); if (r != 71340511u) { fails++; printf("FAIL k35 got %u\n", r); } }
    { uint32_t r = k36(1070406884u); if (r != 27446324u) { fails++; printf("FAIL k36 got %u\n", r); } }
    { uint32_t r = k37(3724842645u); if (r != 93121071u) { fails++; printf("FAIL k37 got %u\n", r); } }
    { uint32_t r = k38(2084311110u); if (r != 50836854u) { fails++; printf("FAIL k38 got %u\n", r); } }
    { uint32_t r = k39(443779575u); if (r != 10566187u) { fails++; printf("FAIL k39 got %u\n", r); } }
    { uint32_t r = k40(3098215336u); if (r != 72051500u) { fails++; printf("FAIL k40 got %u\n", r); } }
    { uint32_t r = k41(1457683801u); if (r != 33129172u) { fails++; printf("FAIL k41 got %u\n", r); } }
    { uint32_t r = k42(4112119562u); if (r != 91380466u) { fails++; printf("FAIL k42 got %u\n", r); } }
    { uint32_t r = k43(2471588027u); if (r != 53730153u) { fails++; printf("FAIL k43 got %u\n", r); } }
    { uint32_t r = k44(831056492u); if (r != 17682052u) { fails++; printf("FAIL k44 got %u\n", r); } }
    { uint32_t r = k45(3485492253u); if (r != 72614456u) { fails++; printf("FAIL k45 got %u\n", r); } }
    { uint32_t r = k46(1844960718u); if (r != 37652280u) { fails++; printf("FAIL k46 got %u\n", r); } }
    { uint32_t r = k47(204429183u); if (r != 4088614u) { fails++; printf("FAIL k47 got %u\n", r); } }
    if (fails == 0) puts("ALL OK");
    return fails;
}

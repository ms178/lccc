/* Allocation-cost corpus for the i686 peephole (MB-20).
 *
 * Deterministic, portable C exercising the two passes whose per-line
 * allocation was removed: the select-diamond -> cmovcc fold and the
 * repeated-DIV staging walk.
 *
 * One select per function is deliberate. Measured on the baseline
 * compiler: a single register-arm ternary folds to cmovcc, but two or
 * more chained selects in one function do not (the extra live values
 * spill, so the arms stop being plain register moves). Many small
 * functions therefore maximise the number of fold attempts, which is
 * what the removed allocation was paid on.
 *
 * Lives in a subdirectory on purpose: tests/regression/*.c is globbed
 * non-recursively and compiled standalone by the SSA corpus runner.
 */

int sel_0(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_1(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_2(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_3(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_4(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_5(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_6(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_7(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_8(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_9(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_10(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_11(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_12(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_13(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_14(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_15(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_16(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_17(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_18(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_19(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_20(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_21(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_22(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_23(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_24(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_25(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_26(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_27(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_28(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_29(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_30(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_31(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_32(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_33(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_34(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_35(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_36(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_37(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_38(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_39(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_40(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_41(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_42(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_43(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_44(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_45(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_46(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_47(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_48(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_49(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_50(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_51(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_52(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_53(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_54(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_55(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_56(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_57(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_58(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_59(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_60(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_61(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_62(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_63(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_64(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_65(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_66(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_67(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_68(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_69(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_70(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_71(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_72(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_73(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_74(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_75(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_76(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_77(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_78(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_79(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_80(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_81(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_82(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_83(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_84(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_85(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_86(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_87(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_88(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_89(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_90(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_91(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_92(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_93(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_94(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_95(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_96(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_97(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_98(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_99(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_100(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_101(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_102(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_103(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_104(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_105(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_106(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_107(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_108(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_109(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_110(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_111(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int sel_112(int a, int b, int c, int d) {
    return c ? a : b;
}

int sel_113(int a, int b, int c, int d) {
    return c > 3 ? a : b;
}

int sel_114(int a, int b, int c, int d) {
    return (a ^ 7) ? a : b;
}

int sel_115(int a, int b, int c, int d) {
    return d ? a : b;
}

int sel_116(int a, int b, int c, int d) {
    return d <= 11 ? a : b;
}

int sel_117(int a, int b, int c, int d) {
    return (b & 1) ? a : b;
}

int sel_118(int a, int b, int c, int d) {
    return c == d ? a : b;
}

int sel_119(int a, int b, int c, int d) {
    return a < b ? a : b;
}

int div_0(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 1) / b;
    return q + r + s;
}

int div_1(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 2) / b;
    return q + r + s;
}

int div_2(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 3) / b;
    return q + r + s;
}

int div_3(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 4) / b;
    return q + r + s;
}

int div_4(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 5) / b;
    return q + r + s;
}

int div_5(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 6) / b;
    return q + r + s;
}

int div_6(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 7) / b;
    return q + r + s;
}

int div_7(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 8) / b;
    return q + r + s;
}

int div_8(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 9) / b;
    return q + r + s;
}

int div_9(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 10) / b;
    return q + r + s;
}

int div_10(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 11) / b;
    return q + r + s;
}

int div_11(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 12) / b;
    return q + r + s;
}

int div_12(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 13) / b;
    return q + r + s;
}

int div_13(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 14) / b;
    return q + r + s;
}

int div_14(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 15) / b;
    return q + r + s;
}

int div_15(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 16) / b;
    return q + r + s;
}

int div_16(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 17) / b;
    return q + r + s;
}

int div_17(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 18) / b;
    return q + r + s;
}

int div_18(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 19) / b;
    return q + r + s;
}

int div_19(int a, int b, int c) {
    int q = a / b;
    int r = a % b;
    int s = (c + 20) / b;
    return q + r + s;
}

int main(void) {
    int acc = 0;
    for (int i = 1; i < 9; ++i) {
        acc += sel_0(acc + i, i * 3, i & 1, i + 0);
        acc += sel_1(acc + i, i * 3, i & 1, i + 1);
        acc += sel_2(acc + i, i * 3, i & 1, i + 2);
        acc += sel_3(acc + i, i * 3, i & 1, i + 3);
        acc += sel_4(acc + i, i * 3, i & 1, i + 4);
        acc += sel_5(acc + i, i * 3, i & 1, i + 5);
        acc += sel_6(acc + i, i * 3, i & 1, i + 6);
        acc += sel_7(acc + i, i * 3, i & 1, i + 7);
        acc += sel_8(acc + i, i * 3, i & 1, i + 8);
        acc += sel_9(acc + i, i * 3, i & 1, i + 9);
        acc += sel_10(acc + i, i * 3, i & 1, i + 10);
        acc += sel_11(acc + i, i * 3, i & 1, i + 11);
        acc += sel_12(acc + i, i * 3, i & 1, i + 12);
        acc += sel_13(acc + i, i * 3, i & 1, i + 13);
        acc += sel_14(acc + i, i * 3, i & 1, i + 14);
        acc += sel_15(acc + i, i * 3, i & 1, i + 15);
        acc += sel_16(acc + i, i * 3, i & 1, i + 16);
        acc += sel_17(acc + i, i * 3, i & 1, i + 17);
        acc += sel_18(acc + i, i * 3, i & 1, i + 18);
        acc += sel_19(acc + i, i * 3, i & 1, i + 19);
        acc += sel_20(acc + i, i * 3, i & 1, i + 20);
        acc += sel_21(acc + i, i * 3, i & 1, i + 21);
        acc += sel_22(acc + i, i * 3, i & 1, i + 22);
        acc += sel_23(acc + i, i * 3, i & 1, i + 23);
        acc += sel_24(acc + i, i * 3, i & 1, i + 24);
        acc += sel_25(acc + i, i * 3, i & 1, i + 25);
        acc += sel_26(acc + i, i * 3, i & 1, i + 26);
        acc += sel_27(acc + i, i * 3, i & 1, i + 27);
        acc += sel_28(acc + i, i * 3, i & 1, i + 28);
        acc += sel_29(acc + i, i * 3, i & 1, i + 29);
        acc += sel_30(acc + i, i * 3, i & 1, i + 30);
        acc += sel_31(acc + i, i * 3, i & 1, i + 31);
        acc += sel_32(acc + i, i * 3, i & 1, i + 32);
        acc += sel_33(acc + i, i * 3, i & 1, i + 33);
        acc += sel_34(acc + i, i * 3, i & 1, i + 34);
        acc += sel_35(acc + i, i * 3, i & 1, i + 35);
        acc += sel_36(acc + i, i * 3, i & 1, i + 36);
        acc += sel_37(acc + i, i * 3, i & 1, i + 37);
        acc += sel_38(acc + i, i * 3, i & 1, i + 38);
        acc += sel_39(acc + i, i * 3, i & 1, i + 39);
        acc += sel_40(acc + i, i * 3, i & 1, i + 40);
        acc += sel_41(acc + i, i * 3, i & 1, i + 41);
        acc += sel_42(acc + i, i * 3, i & 1, i + 42);
        acc += sel_43(acc + i, i * 3, i & 1, i + 43);
        acc += sel_44(acc + i, i * 3, i & 1, i + 44);
        acc += sel_45(acc + i, i * 3, i & 1, i + 45);
        acc += sel_46(acc + i, i * 3, i & 1, i + 46);
        acc += sel_47(acc + i, i * 3, i & 1, i + 47);
        acc += sel_48(acc + i, i * 3, i & 1, i + 48);
        acc += sel_49(acc + i, i * 3, i & 1, i + 49);
        acc += sel_50(acc + i, i * 3, i & 1, i + 50);
        acc += sel_51(acc + i, i * 3, i & 1, i + 51);
        acc += sel_52(acc + i, i * 3, i & 1, i + 52);
        acc += sel_53(acc + i, i * 3, i & 1, i + 53);
        acc += sel_54(acc + i, i * 3, i & 1, i + 54);
        acc += sel_55(acc + i, i * 3, i & 1, i + 55);
        acc += sel_56(acc + i, i * 3, i & 1, i + 56);
        acc += sel_57(acc + i, i * 3, i & 1, i + 57);
        acc += sel_58(acc + i, i * 3, i & 1, i + 58);
        acc += sel_59(acc + i, i * 3, i & 1, i + 59);
        acc += sel_60(acc + i, i * 3, i & 1, i + 60);
        acc += sel_61(acc + i, i * 3, i & 1, i + 61);
        acc += sel_62(acc + i, i * 3, i & 1, i + 62);
        acc += sel_63(acc + i, i * 3, i & 1, i + 63);
        acc += sel_64(acc + i, i * 3, i & 1, i + 64);
        acc += sel_65(acc + i, i * 3, i & 1, i + 65);
        acc += sel_66(acc + i, i * 3, i & 1, i + 66);
        acc += sel_67(acc + i, i * 3, i & 1, i + 67);
        acc += sel_68(acc + i, i * 3, i & 1, i + 68);
        acc += sel_69(acc + i, i * 3, i & 1, i + 69);
        acc += sel_70(acc + i, i * 3, i & 1, i + 70);
        acc += sel_71(acc + i, i * 3, i & 1, i + 71);
        acc += sel_72(acc + i, i * 3, i & 1, i + 72);
        acc += sel_73(acc + i, i * 3, i & 1, i + 73);
        acc += sel_74(acc + i, i * 3, i & 1, i + 74);
        acc += sel_75(acc + i, i * 3, i & 1, i + 75);
        acc += sel_76(acc + i, i * 3, i & 1, i + 76);
        acc += sel_77(acc + i, i * 3, i & 1, i + 77);
        acc += sel_78(acc + i, i * 3, i & 1, i + 78);
        acc += sel_79(acc + i, i * 3, i & 1, i + 79);
        acc += sel_80(acc + i, i * 3, i & 1, i + 80);
        acc += sel_81(acc + i, i * 3, i & 1, i + 81);
        acc += sel_82(acc + i, i * 3, i & 1, i + 82);
        acc += sel_83(acc + i, i * 3, i & 1, i + 83);
        acc += sel_84(acc + i, i * 3, i & 1, i + 84);
        acc += sel_85(acc + i, i * 3, i & 1, i + 85);
        acc += sel_86(acc + i, i * 3, i & 1, i + 86);
        acc += sel_87(acc + i, i * 3, i & 1, i + 87);
        acc += sel_88(acc + i, i * 3, i & 1, i + 88);
        acc += sel_89(acc + i, i * 3, i & 1, i + 89);
        acc += sel_90(acc + i, i * 3, i & 1, i + 90);
        acc += sel_91(acc + i, i * 3, i & 1, i + 91);
        acc += sel_92(acc + i, i * 3, i & 1, i + 92);
        acc += sel_93(acc + i, i * 3, i & 1, i + 93);
        acc += sel_94(acc + i, i * 3, i & 1, i + 94);
        acc += sel_95(acc + i, i * 3, i & 1, i + 95);
        acc += sel_96(acc + i, i * 3, i & 1, i + 96);
        acc += sel_97(acc + i, i * 3, i & 1, i + 97);
        acc += sel_98(acc + i, i * 3, i & 1, i + 98);
        acc += sel_99(acc + i, i * 3, i & 1, i + 99);
        acc += sel_100(acc + i, i * 3, i & 1, i + 100);
        acc += sel_101(acc + i, i * 3, i & 1, i + 101);
        acc += sel_102(acc + i, i * 3, i & 1, i + 102);
        acc += sel_103(acc + i, i * 3, i & 1, i + 103);
        acc += sel_104(acc + i, i * 3, i & 1, i + 104);
        acc += sel_105(acc + i, i * 3, i & 1, i + 105);
        acc += sel_106(acc + i, i * 3, i & 1, i + 106);
        acc += sel_107(acc + i, i * 3, i & 1, i + 107);
        acc += sel_108(acc + i, i * 3, i & 1, i + 108);
        acc += sel_109(acc + i, i * 3, i & 1, i + 109);
        acc += sel_110(acc + i, i * 3, i & 1, i + 110);
        acc += sel_111(acc + i, i * 3, i & 1, i + 111);
        acc += sel_112(acc + i, i * 3, i & 1, i + 112);
        acc += sel_113(acc + i, i * 3, i & 1, i + 113);
        acc += sel_114(acc + i, i * 3, i & 1, i + 114);
        acc += sel_115(acc + i, i * 3, i & 1, i + 115);
        acc += sel_116(acc + i, i * 3, i & 1, i + 116);
        acc += sel_117(acc + i, i * 3, i & 1, i + 117);
        acc += sel_118(acc + i, i * 3, i & 1, i + 118);
        acc += sel_119(acc + i, i * 3, i & 1, i + 119);
        acc += div_0(acc + 100, i + 1, i * 7);
        acc += div_1(acc + 100, i + 1, i * 7);
        acc += div_2(acc + 100, i + 1, i * 7);
        acc += div_3(acc + 100, i + 1, i * 7);
        acc += div_4(acc + 100, i + 1, i * 7);
        acc += div_5(acc + 100, i + 1, i * 7);
        acc += div_6(acc + 100, i + 1, i * 7);
        acc += div_7(acc + 100, i + 1, i * 7);
        acc += div_8(acc + 100, i + 1, i * 7);
        acc += div_9(acc + 100, i + 1, i * 7);
        acc += div_10(acc + 100, i + 1, i * 7);
        acc += div_11(acc + 100, i + 1, i * 7);
        acc += div_12(acc + 100, i + 1, i * 7);
        acc += div_13(acc + 100, i + 1, i * 7);
        acc += div_14(acc + 100, i + 1, i * 7);
        acc += div_15(acc + 100, i + 1, i * 7);
        acc += div_16(acc + 100, i + 1, i * 7);
        acc += div_17(acc + 100, i + 1, i * 7);
        acc += div_18(acc + 100, i + 1, i * 7);
        acc += div_19(acc + 100, i + 1, i * 7);
    }
    return acc == 0;
}

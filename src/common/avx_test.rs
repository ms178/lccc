//! AVX VTESTPS/PD and 256-bit VPTEST contracts. VTEST observes only each FP
//! lane's sign bit; VPTEST observes every payload bit. ZF tests (a & b), CF
//! tests (~a & b); `testnzc` requires both flags clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AvxTestCondition {
    Zero,
    Carry,
    Neither,
}

/// Which bits participate: FP sign bits per 32/64-bit lane, or all bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AvxTestElem {
    Ps,
    Pd,
    Int,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AvxTest {
    pub elem: AvxTestElem,
    pub wide: bool,
    pub condition: AvxTestCondition,
}

impl AvxTest {
    pub fn width(self) -> usize {
        if self.wide { 32 } else { 16 }
    }

    pub fn mnemonic(self) -> &'static str {
        match self.elem {
            AvxTestElem::Ps => "vtestps",
            AvxTestElem::Pd => "vtestpd",
            AvxTestElem::Int => "vptest",
        }
    }

    pub fn setcc(self) -> &'static str {
        match self.condition {
            AvxTestCondition::Zero => "sete",
            AvxTestCondition::Carry => "setb",
            AvxTestCondition::Neither => "seta",
        }
    }
}

/// Public API and GCC builtin spellings share one typed operand contract.
pub const AVX_TESTS: &[(&str, &str, &str, AvxTest)] = &[
    (
        "_mm_testz_ps",
        "__builtin_ia32_vtestzps",
        "__lccc_avx_testzps128",
        AvxTest {
            elem: AvxTestElem::Ps,
            wide: false,
            condition: AvxTestCondition::Zero,
        },
    ),
    (
        "_mm_testc_ps",
        "__builtin_ia32_vtestcps",
        "__lccc_avx_testcps128",
        AvxTest {
            elem: AvxTestElem::Ps,
            wide: false,
            condition: AvxTestCondition::Carry,
        },
    ),
    (
        "_mm_testnzc_ps",
        "__builtin_ia32_vtestnzcps",
        "__lccc_avx_testnzcps128",
        AvxTest {
            elem: AvxTestElem::Ps,
            wide: false,
            condition: AvxTestCondition::Neither,
        },
    ),
    (
        "_mm_testz_pd",
        "__builtin_ia32_vtestzpd",
        "__lccc_avx_testzpd128",
        AvxTest {
            elem: AvxTestElem::Pd,
            wide: false,
            condition: AvxTestCondition::Zero,
        },
    ),
    (
        "_mm_testc_pd",
        "__builtin_ia32_vtestcpd",
        "__lccc_avx_testcpd128",
        AvxTest {
            elem: AvxTestElem::Pd,
            wide: false,
            condition: AvxTestCondition::Carry,
        },
    ),
    (
        "_mm_testnzc_pd",
        "__builtin_ia32_vtestnzcpd",
        "__lccc_avx_testnzcpd128",
        AvxTest {
            elem: AvxTestElem::Pd,
            wide: false,
            condition: AvxTestCondition::Neither,
        },
    ),
    (
        "_mm256_testz_ps",
        "__builtin_ia32_vtestzps256",
        "__lccc_avx_testzps256",
        AvxTest {
            elem: AvxTestElem::Ps,
            wide: true,
            condition: AvxTestCondition::Zero,
        },
    ),
    (
        "_mm256_testc_ps",
        "__builtin_ia32_vtestcps256",
        "__lccc_avx_testcps256",
        AvxTest {
            elem: AvxTestElem::Ps,
            wide: true,
            condition: AvxTestCondition::Carry,
        },
    ),
    (
        "_mm256_testnzc_ps",
        "__builtin_ia32_vtestnzcps256",
        "__lccc_avx_testnzcps256",
        AvxTest {
            elem: AvxTestElem::Ps,
            wide: true,
            condition: AvxTestCondition::Neither,
        },
    ),
    (
        "_mm256_testz_pd",
        "__builtin_ia32_vtestzpd256",
        "__lccc_avx_testzpd256",
        AvxTest {
            elem: AvxTestElem::Pd,
            wide: true,
            condition: AvxTestCondition::Zero,
        },
    ),
    (
        "_mm256_testc_pd",
        "__builtin_ia32_vtestcpd256",
        "__lccc_avx_testcpd256",
        AvxTest {
            elem: AvxTestElem::Pd,
            wide: true,
            condition: AvxTestCondition::Carry,
        },
    ),
    (
        "_mm256_testnzc_pd",
        "__builtin_ia32_vtestnzcpd256",
        "__lccc_avx_testnzcpd256",
        AvxTest {
            elem: AvxTestElem::Pd,
            wide: true,
            condition: AvxTestCondition::Neither,
        },
    ),
    // AVX (not AVX2) VEX.256 PTEST; the 128-bit `_mm_test*_si128` forms are
    // SSE4.1 and live in <smmintrin.h>.
    (
        "_mm256_testz_si256",
        "__builtin_ia32_ptestz256",
        "__lccc_avx_testzsi256",
        AvxTest {
            elem: AvxTestElem::Int,
            wide: true,
            condition: AvxTestCondition::Zero,
        },
    ),
    (
        "_mm256_testc_si256",
        "__builtin_ia32_ptestc256",
        "__lccc_avx_testcsi256",
        AvxTest {
            elem: AvxTestElem::Int,
            wide: true,
            condition: AvxTestCondition::Carry,
        },
    ),
    (
        "_mm256_testnzc_si256",
        "__builtin_ia32_ptestnzc256",
        "__lccc_avx_testnzcsi256",
        AvxTest {
            elem: AvxTestElem::Int,
            wide: true,
            condition: AvxTestCondition::Neither,
        },
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_cartesian_product_and_unique_spellings() {
        assert_eq!(AVX_TESTS.len(), 15);
        for (index, &(public, raw, private, test)) in AVX_TESTS.iter().enumerate() {
            assert_eq!(test.width(), if test.wide { 32 } else { 16 });
            let (mnemonic, suffix) = match test.elem {
                AvxTestElem::Ps => ("vtestps", "_ps"),
                AvxTestElem::Pd => ("vtestpd", "_pd"),
                AvxTestElem::Int => ("vptest", "_si256"),
            };
            assert_eq!(test.mnemonic(), mnemonic);
            assert!(public.ends_with(suffix), "{public}");
            // Integer PTEST exists only as the 256-bit AVX form here.
            assert!(test.elem != AvxTestElem::Int || test.wide, "{public}");
            for &(other_public, other_raw, other_private, other_test) in &AVX_TESTS[index + 1..] {
                assert_ne!(public, other_public);
                assert_ne!(raw, other_raw);
                assert_ne!(private, other_private);
                assert_ne!(test, other_test);
            }
        }
    }

    #[test]
    fn flag_conditions_are_unsigned_and_not_inverted() {
        let base = AvxTest {
            elem: AvxTestElem::Ps,
            wide: false,
            condition: AvxTestCondition::Zero,
        };
        assert_eq!(base.setcc(), "sete");
        assert_eq!(
            AvxTest {
                condition: AvxTestCondition::Carry,
                ..base
            }
            .setcc(),
            "setb"
        );
        assert_eq!(
            AvxTest {
                condition: AvxTestCondition::Neither,
                ..base
            }
            .setcc(),
            "seta"
        );
    }
}

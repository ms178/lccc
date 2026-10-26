//! AVX VTESTPS/PD contracts: only each FP lane's sign bit participates.
//! ZF tests (a & b), CF tests (~a & b); `testnzc` requires both flags clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AvxTestCondition {
    Zero,
    Carry,
    Neither,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AvxTest {
    pub double: bool,
    pub wide: bool,
    pub condition: AvxTestCondition,
}

impl AvxTest {
    pub fn width(self) -> usize {
        if self.wide { 32 } else { 16 }
    }

    pub fn mnemonic(self) -> &'static str {
        if self.double { "vtestpd" } else { "vtestps" }
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
            double: false,
            wide: false,
            condition: AvxTestCondition::Zero,
        },
    ),
    (
        "_mm_testc_ps",
        "__builtin_ia32_vtestcps",
        "__lccc_avx_testcps128",
        AvxTest {
            double: false,
            wide: false,
            condition: AvxTestCondition::Carry,
        },
    ),
    (
        "_mm_testnzc_ps",
        "__builtin_ia32_vtestnzcps",
        "__lccc_avx_testnzcps128",
        AvxTest {
            double: false,
            wide: false,
            condition: AvxTestCondition::Neither,
        },
    ),
    (
        "_mm_testz_pd",
        "__builtin_ia32_vtestzpd",
        "__lccc_avx_testzpd128",
        AvxTest {
            double: true,
            wide: false,
            condition: AvxTestCondition::Zero,
        },
    ),
    (
        "_mm_testc_pd",
        "__builtin_ia32_vtestcpd",
        "__lccc_avx_testcpd128",
        AvxTest {
            double: true,
            wide: false,
            condition: AvxTestCondition::Carry,
        },
    ),
    (
        "_mm_testnzc_pd",
        "__builtin_ia32_vtestnzcpd",
        "__lccc_avx_testnzcpd128",
        AvxTest {
            double: true,
            wide: false,
            condition: AvxTestCondition::Neither,
        },
    ),
    (
        "_mm256_testz_ps",
        "__builtin_ia32_vtestzps256",
        "__lccc_avx_testzps256",
        AvxTest {
            double: false,
            wide: true,
            condition: AvxTestCondition::Zero,
        },
    ),
    (
        "_mm256_testc_ps",
        "__builtin_ia32_vtestcps256",
        "__lccc_avx_testcps256",
        AvxTest {
            double: false,
            wide: true,
            condition: AvxTestCondition::Carry,
        },
    ),
    (
        "_mm256_testnzc_ps",
        "__builtin_ia32_vtestnzcps256",
        "__lccc_avx_testnzcps256",
        AvxTest {
            double: false,
            wide: true,
            condition: AvxTestCondition::Neither,
        },
    ),
    (
        "_mm256_testz_pd",
        "__builtin_ia32_vtestzpd256",
        "__lccc_avx_testzpd256",
        AvxTest {
            double: true,
            wide: true,
            condition: AvxTestCondition::Zero,
        },
    ),
    (
        "_mm256_testc_pd",
        "__builtin_ia32_vtestcpd256",
        "__lccc_avx_testcpd256",
        AvxTest {
            double: true,
            wide: true,
            condition: AvxTestCondition::Carry,
        },
    ),
    (
        "_mm256_testnzc_pd",
        "__builtin_ia32_vtestnzcpd256",
        "__lccc_avx_testnzcpd256",
        AvxTest {
            double: true,
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
        assert_eq!(AVX_TESTS.len(), 12);
        for (index, &(public, raw, private, test)) in AVX_TESTS.iter().enumerate() {
            assert_eq!(test.width(), if test.wide { 32 } else { 16 });
            assert_eq!(
                test.mnemonic(),
                if test.double { "vtestpd" } else { "vtestps" }
            );
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
            double: false,
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

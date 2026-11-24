//! `-march=native`: the CPU row the executing host grants.
//!
//! Lives outside `arch_table.rs`, which is generated from the GCC-measured
//! table. Mapping the probe's CPUID names onto lccc's features is hand logic,
//! so it sits here where it can be reviewed as code.

use super::arch_table::{ABM, X86ArchRow, X86ExtraFeature};
use crate::common::x86_features::X86Feature;

impl X86ArchRow {
    /// The row `-march=native` grants: the names the executing CPU reports
    /// (`common::cpu_probe::host_supported_names`), mapped onto the modelled
    /// features and macro-only extensions. A name the driver does not model
    /// (`sse4a`, `xop`, `tbm`, ...) has no `-m` flag and is dropped here.
    /// Every entry is a CPU fact, so the row is granted exactly like a table
    /// row and never counts as an explicit option.
    #[must_use]
    pub(crate) fn host(names: &[&str]) -> X86ArchRow {
        let mut features = 0u64;
        let mut extras = 0u64;
        for name in names {
            // The probe calls LAHF/SAHF in long mode by its CPUID name; the
            // option, and with it the extension, is `sahf`.
            let (option, probe) = crate::common::cpu_probe::SAHF_OPTION_AND_PROBE;
            let name = if *name == probe { option } else { name };
            if let Some(feature) = X86Feature::parse(name) {
                features |= 1 << feature as u8;
            } else if let Some(extra) = X86ExtraFeature::ALL
                .into_iter()
                .find(|extra| extra.option() == name)
            {
                extras |= 1 << extra as u8;
            }
        }
        // ABM is CPUID.0x80000001:ECX[5], the same bit as LZCNT (the probe's
        // `abm` alias is `lzcnt`, `cpu_probe::NAME_ALIASES`). GCC's native
        // detection enables `-mabm` from that bit, so `__ABM__` follows LZCNT
        // here (measured, gcc-16.2 `-march=native` on an Ice Lake host).
        if names.contains(&"lzcnt") {
            features |= ABM;
        }
        X86ArchRow {
            name: "native",
            x86_64: true,
            features,
            extras,
            arch_macros: &[],
            tune_macros: &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{X86ArchRow, X86ExtraFeature, X86Feature};

    /// The probe's names reach the extension of the same option name, and
    /// LAHF/SAHF, which the probe knows by its CPUID name, reaches `sahf`.
    #[test]
    fn host_names_reach_features_and_extensions() {
        let row = X86ArchRow::host(&["avx2", "adx", "rdseed", "lahf-lm", "xsaves", "clwb"]);
        assert!(row.has(X86Feature::Avx2));
        for extra in [
            X86ExtraFeature::Adx,
            X86ExtraFeature::Rdseed,
            X86ExtraFeature::Sahf,
            X86ExtraFeature::Xsaves,
            X86ExtraFeature::Clwb,
        ] {
            assert!(row.has_extra(extra), "{extra:?}");
        }
        assert!(!row.has_extra(X86ExtraFeature::Rdpid));
        // A name nothing models is dropped, not an error.
        let row = X86ArchRow::host(&["xop", "tbm", "no-such-thing"]);
        assert_eq!((row.features, row.extras), (0, 0));
    }

    /// Every extension has a bit of the row's `u64`.
    #[test]
    fn every_extension_fits_the_row() {
        assert!(X86ExtraFeature::ALL.len() <= 64);
        let all: Vec<&str> = X86ExtraFeature::ALL.iter().map(|e| e.option()).collect();
        let row = X86ArchRow::host(&all);
        assert_eq!(row.extras.count_ones() as usize, X86ExtraFeature::ALL.len());
    }
}

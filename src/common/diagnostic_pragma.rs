//! Shared GCC diagnostic pragma state for preprocessing and span-based diagnostics.
use super::error::WarningConfig;

enum Action<'a> {
    Push,
    Pop,
    Set { level: &'a str, name: &'a str },
}

fn parse(text: &str) -> Option<Action<'_>> {
    let rest = text.trim().strip_prefix("GCC")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start().strip_prefix("diagnostic")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim();
    match rest {
        "push" => Some(Action::Push),
        "pop" => Some(Action::Pop),
        _ => {
            let (level, option) = rest.split_once(char::is_whitespace)?;
            if !matches!(level, "ignored" | "warning" | "error") {
                return None;
            }
            let name = option
                .trim()
                .strip_prefix('"')?
                .strip_suffix('"')?
                .strip_prefix("-W")?;
            Some(Action::Set { level, name })
        }
    }
}

/// Threshold-setting pragmas need an argument just like their CLI spelling.
/// Ignoring the family does not inspect/change the numeric threshold.
pub(crate) fn threshold_error(text: &str) -> Option<String> {
    let Action::Set { level, name } = parse(text)? else {
        return None;
    };
    if level == "ignored" {
        return None;
    }
    for family in ["frame-larger-than", "vla-larger-than"] {
        if let Some(value) = name.strip_prefix(family).and_then(|s| s.strip_prefix('='))
            && WarningConfig::byte_threshold(value).is_none()
        {
            return Some(format!("invalid or missing argument to '-W{family}='"));
        }
    }
    None
}

pub(crate) fn is_diagnostic_pragma(text: &str) -> bool {
    parse(text).is_some()
}

pub(crate) struct DiagnosticPragmas {
    baseline: WarningConfig,
    pub(crate) current: WarningConfig,
    /// GCC enables analysis with positive classifications; ignored/pop change
    /// reporting only, not these sticky flags (important for mutually exclusive
    /// missing-prototypes/missing-declarations analyses).
    pub(crate) analysis: WarningConfig,
    stack: Vec<WarningConfig>,
}

impl DiagnosticPragmas {
    pub(crate) fn new(baseline: WarningConfig) -> Self {
        Self {
            current: baseline.clone(),
            analysis: baseline.clone(),
            baseline,
            stack: Vec::new(),
        }
    }

    /// `text` excludes #pragma. Option strings are not macro expanded.
    pub(crate) fn apply(&mut self, text: &str) -> bool {
        let Some(action) = parse(text) else {
            return false;
        };
        match action {
            Action::Push => self.stack.push(self.current.clone()),
            Action::Pop => self.current = self.stack.pop().unwrap_or_else(|| self.baseline.clone()),
            Action::Set {
                level: "ignored",
                name,
            } => {
                let name = match name.split_once('=') {
                    Some((base @ ("frame-larger-than" | "vla-larger-than"), _)) => base,
                    _ => name,
                };
                self.current.process_flag(&format!("no-{name}"));
            }
            Action::Set { level, name } => {
                let name = if name == "frame-larger-than=" {
                    "frame-larger-than"
                } else {
                    name
                };
                self.analysis.process_flag(name);
                if level == "warning" {
                    self.current.process_flag(name);
                    self.current.process_flag(&format!("no-error={name}"));
                } else {
                    self.current.process_flag(&format!("error={name}"));
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod threshold_tests {
    use super::*;
    use crate::common::error::WarningKind;

    #[test]
    fn ignored_thresholds_accept_empty_and_invalid_numbers_and_pop_restores() {
        for (family, kind) in [
            ("frame-larger-than", WarningKind::FrameLargerThan),
            ("vla-larger-than", WarningKind::VlaLargerThan),
        ] {
            let mut config = WarningConfig::default();
            assert!(config.process_flag(&format!("{family}=2048")));
            let mut pragmas = DiagnosticPragmas::new(config);
            for value in ["", "nonsense", "999999999999999999999999999"] {
                pragmas.apply("GCC diagnostic push");
                let text = format!("GCC diagnostic ignored \"-W{family}={value}\"");
                assert!(threshold_error(&text).is_none());
                assert!(pragmas.apply(&text));
                assert!(!pragmas.current.is_enabled(kind));
                pragmas.apply("GCC diagnostic pop");
                assert!(pragmas.current.is_enabled(kind));
            }
        }
    }
    #[test]
    fn positive_thresholds_validate_and_saturate() {
        for family in ["frame-larger-than", "vla-larger-than"] {
            for level in ["warning", "error"] {
                for bad in ["", "+1", "1ZB", " 1"] {
                    assert!(
                        threshold_error(&format!("GCC diagnostic {level} \"-W{family}={bad}\""))
                            .is_some()
                    );
                }
                assert!(
                    threshold_error(&format!(
                        "GCC diagnostic {level} \"-W{family}=999999999999999999999999999\""
                    ))
                    .is_none()
                );
            }
        }
        assert_eq!(
            WarningConfig::byte_threshold("999999999999999999999999999"),
            Some(u64::MAX)
        );
        assert_eq!(WarningConfig::byte_threshold("99999EiB"), Some(u64::MAX));
        assert_eq!(WarningConfig::byte_threshold("2KiB"), Some(2048));
    }
}

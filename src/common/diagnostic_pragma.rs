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
                self.current.process_flag(&format!("no-{name}"));
            }
            Action::Set { level, name } => {
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

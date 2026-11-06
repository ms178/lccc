//! Ordered source-path prefix rewriting shared by the driver and DWARF writer.
//!
//! GNU applies the first matching `-f*-prefix-map` entry, not the longest
//! match.  Keeping that order in one type prevents debug paths and `__FILE__`
//! from disagreeing when prefixes overlap.

use std::borrow::Cow;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PathPrefixMap {
    entries: Vec<(String, String)>,
}

impl PathPrefixMap {
    pub(crate) fn push(&mut self, prefix: String, replacement: String) {
        self.entries.push((prefix, replacement));
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn apply<'a>(&self, path: &'a str) -> Cow<'a, str> {
        for (prefix, replacement) in &self.entries {
            if let Some(suffix) = path.strip_prefix(prefix.as_str()) {
                let mut result = String::with_capacity(replacement.len() + suffix.len());
                result.push_str(replacement);
                result.push_str(suffix);
                return Cow::Owned(result);
            }
        }
        Cow::Borrowed(path)
    }
}

#[cfg(test)]
mod tests {
    use super::PathPrefixMap;

    #[test]
    fn first_match_wins_and_replacement_is_literal() {
        let mut map = PathPrefixMap::default();
        assert_eq!(map.apply("/build/x.c"), "/build/x.c");
        map.push("/build".into(), "/src$1\\".into());
        map.push("/build/private".into(), "WRONG".into());
        assert_eq!(map.apply("/build/private/x.c"), "/src$1\\/private/x.c");
        assert_eq!(map.apply("/elsewhere/x.c"), "/elsewhere/x.c");
    }

    #[test]
    fn empty_prefix_and_empty_replacement_are_valid() {
        let mut map = PathPrefixMap::default();
        map.push("/build/".into(), String::new());
        map.push(String::new(), "fallback".into());
        assert_eq!(map.apply("/build/x.c"), "x.c");
        assert_eq!(map.apply("x.c"), "fallbackx.c");
    }
}

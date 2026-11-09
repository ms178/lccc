pub(crate) mod assertions;
pub(crate) mod builtin_macros;
pub(crate) mod conditionals;
mod expr_eval;
mod feature_queries;
mod includes;
pub(crate) mod macro_defs;
pub(crate) mod pipeline;
pub(crate) mod pragmas;
pub(crate) mod predefined_macros;
mod text_processing;
pub(crate) mod utils;

pub(crate) use pipeline::{PragmaSeverity, Preprocessor};

#[cfg(test)]
mod physical_line_tests;

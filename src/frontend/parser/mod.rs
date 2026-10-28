pub(crate) mod asm_validate;
pub(crate) mod ast;
mod attribute_registry;
mod declarations;
mod declarators;
mod expressions;
mod nested_functions;
pub(crate) mod parse;
mod statements;
mod types;

pub(crate) use parse::Parser;

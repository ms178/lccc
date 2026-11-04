pub(crate) mod analysis;
pub(crate) mod builtins;
pub(crate) mod const_eval;
pub(crate) mod gcc16_builtins;
pub(crate) mod ice;
mod init_cursor;
pub(crate) mod intrinsic_arity;
pub(crate) mod label_check;
mod operand_constraints;
pub(crate) mod static_init;
pub(crate) mod stdbit;
pub(crate) mod type_checker;
pub(crate) mod type_context;
pub(crate) mod void_declarators;

pub(crate) use analysis::{ExprTypeMap, FunctionInfo, SemanticAnalyzer};
pub(crate) use const_eval::ConstMap;

mod function_pointer_cast;

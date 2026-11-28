pub(crate) mod analysis;
pub(crate) mod builtins;
pub(crate) mod const_eval;
pub(crate) mod gcc16_builtins;
pub(crate) mod ice;
mod init_cursor;
pub(crate) mod intrinsic_arity;
pub(crate) mod label_check;
pub(crate) mod libc_builtin_arity;
mod operand_constraints;
pub(crate) mod static_init;
pub(crate) mod stdbit;
pub(crate) mod type_checker;
pub(crate) mod type_context;
pub(crate) mod typeof_recovery;
pub(crate) mod void_declarators;

pub(crate) use analysis::{ExprTypeMap, FunctionInfo, SemanticAnalyzer};
pub(crate) use const_eval::ConstMap;
pub(crate) use operand_constraints::{integer_operands_only, invalid_binary_operands};

mod function_pointer_cast;

//! S13: `-finstrument-functions` support.
//!
//! GCC contract: when compiling with `-finstrument-functions`, every function
//! DEFINED in the translation unit calls the profiling hooks
//!
//! ```c
//! void __cyg_profile_func_enter (void *this_fn, void *call_site);
//! void __cyg_profile_func_exit  (void *this_fn, void *call_site);
//! ```
//!
//! immediately after entry and immediately before every return, unless the
//! function carries `no_instrument_function` (the front end already parses
//! that attribute into `IrFunction::no_instrument`; this pass is the missing
//! consumer — the plumbing existed, the instrumentation itself did not).
//!
//! Argument contract: argument 1 is the instrumented function's own address
//! (GlobalAddr of the function symbol; the eeprof torture suite checks hook
//! identity exactly — `last_fn_entered == foo`). Argument 2 is the call site;
//! GCC passes the caller-side return address, which the IR does not model,
//! so LCCC passes the function's own address for it as well. The torture
//! suite and the glibc/valgrind-style consumers key on argument 1 only.
//!
//! Placement: the module runs through this pass BEFORE the optimizer, so the
//! hook calls exist at every optimization level, are register-allocated like
//! any other call, and are preserved by DSE/DCE (they are non-pure calls to
//! an extern symbol).
//!
//! Kill switch: `CCC_NO_INSTRUMENT=1`.
use crate::common::types::IrType;
use crate::ir::module::{IrFunction, IrModule};
use crate::ir::reexports::{CallInfo, Instruction, Operand, Terminator, Value};

const HOOK_ENTER: &str = "__cyg_profile_func_enter";
const HOOK_EXIT: &str = "__cyg_profile_func_exit";

/// Instrument every defined, non-exempt function in the module.
pub fn run(module: &mut IrModule) {
    if std::env::var_os("CCC_NO_INSTRUMENT").is_some() {
        return;
    }
    for func in &mut module.functions {
        if func.is_declaration || func.blocks.is_empty() || func.no_instrument {
            continue;
        }
        // The profiling hooks themselves are the instrumentation machinery:
        // instrumenting them recurses without bound (enter calls enter, …).
        // GCC reaches the same result through the NOCHK attribute; the
        // exclusion here additionally covers translation units whose
        // no_instrument_function lives on a declaration that the definition
        // does not re-state.
        if func.name == HOOK_ENTER || func.name == HOOK_EXIT {
            continue;
        }
        instrument_function(func);
    }
}

fn instrument_function(func: &mut IrFunction) {
    // Fresh value ids for the GlobalAddr results and the (unused) call
    // results. next_value_id is the cached upper bound; allocating from it
    // and storing the new bound back keeps DCE's def_loc sizing exact
    // (the fortify_fold next_value_id lesson).
    let mut id = if func.next_value_id > 0 {
        func.next_value_id
    } else {
        func.max_value_id()
    };

    // Entry hook: after the leading ParamRef block of the entry block, so
    // the ABI-visible parameter spills happen first (matches GCC, which
    // instruments after the prologue).
    let mut enter_insts = Vec::with_capacity(3);
    let vfn = Value(id);
    id += 1;
    enter_insts.push(Instruction::GlobalAddr {
        dest: vfn,
        name: func.name.clone(),
    });
    let vsite = Value(id);
    id += 1;
    enter_insts.push(Instruction::GlobalAddr {
        dest: vsite,
        name: func.name.clone(),
    });
    enter_insts.push(Instruction::Call {
        func: HOOK_ENTER.to_string(),
        info: hook_call(vfn, vsite),
    });

    {
        let entry = &mut func.blocks[0];
        let insert_at = entry
            .instructions
            .iter()
            .take_while(|inst| matches!(inst, Instruction::ParamRef { .. }))
            .count();
        let tail = entry.instructions.split_off(insert_at);
        entry.instructions.extend(enter_insts);
        entry.instructions.extend(tail);
    }

    // Exit hooks: immediately before every Return terminator.
    for block in &mut func.blocks {
        let is_return = matches!(block.terminator, Terminator::Return(_));
        if !is_return {
            continue;
        }
        let vfn2 = Value(id);
        id += 1;
        let mut insts = Vec::with_capacity(3);
        insts.push(Instruction::GlobalAddr {
            dest: vfn2,
            name: func.name.clone(),
        });
        let vsite2 = Value(id);
        id += 1;
        insts.push(Instruction::GlobalAddr {
            dest: vsite2,
            name: func.name.clone(),
        });
        insts.push(Instruction::Call {
            func: HOOK_EXIT.to_string(),
            info: hook_call(vfn2, vsite2),
        });
        // Append: the hook must run AFTER the block's effects, immediately
        // before the Return terminator (which lives in `block.terminator`).
        block.instructions.extend(insts);
    }

    func.next_value_id = id;
}

/// Build the CallInfo for a `hook(this_fn, call_site)` invocation.
fn hook_call(vfn: Value, vsite: Value) -> CallInfo {
    CallInfo {
        dest: None, // void hook
        args: vec![Operand::Value(vfn), Operand::Value(vsite)],
        arg_types: vec![IrType::Ptr, IrType::Ptr],
        return_type: IrType::Void,
        is_variadic: false,
        num_fixed_args: 2,
        struct_arg_sizes: vec![None, None],
        struct_arg_aligns: vec![None, None],
        struct_arg_classes: vec![vec![], vec![]],
        struct_arg_riscv_float_classes: vec![None],
        struct_arg_is_f128_sse: vec![false, false],
        ret_is_f128_sse: false,
        is_sret: false,
        is_fastcall: false,
        // The hooks observe and mutate profiler state: never pure, never
        // const, never removable. (Explicit for clarity; these are the
        // defaults an extern call gets anyway.)
        is_pure: false,
        is_const: false,
        ret_eightbyte_classes: vec![],
    }
}

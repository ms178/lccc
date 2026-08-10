//! Profile-guided inlining with a bounded, frequency-aware budget.
use crate::ir::reexports::IrFunction;
use crate::pgo::ProfileData;
fn size_opt() -> bool {
    std::env::var("CFLAGS")
        .map(|s| s.contains("-Os") || s.contains("-Oz"))
        .unwrap_or(false)
}
pub fn inline_threshold_multiplier(caller: &str, callee: &str, p: &ProfileData) -> f64 {
    let cf = p.relative_frequency(caller);
    let df = p.relative_frequency(callee);
    if cf >= 0.10 && df >= 0.05 {
        1.75
    } else if cf >= 0.10 && df < 0.005 {
        0.60
    } else if cf < 0.005 {
        0.80
    } else {
        1.0
    }
}
pub fn should_inline_normal(
    caller: &IrFunction,
    callee: &IrFunction,
    p: Option<&ProfileData>,
) -> Option<bool> {
    let p = p?;
    let cf = p.relative_frequency(&caller.name);
    let df = p.relative_frequency(&callee.name);
    let size: usize = callee.blocks.iter().map(|b| b.instructions.len()).sum();
    if size_opt() {
        if cf >= 0.10 && df >= 0.10 && size <= 12 {
            return Some(true);
        }
        if df < 0.005 && size > 12 {
            return Some(false);
        }
        return None;
    }
    // Profile is a hard force only for tiny hot callees. Medium functions stay
    // under the normal allocator's size budget; this prevents PGO binary bloat.
    if cf >= 0.10 && df >= 0.05 && size <= 32 {
        return Some(true);
    }
    if df < 0.001 && size > 32 {
        return Some(false);
    }
    None
}

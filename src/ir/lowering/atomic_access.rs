//! Accesses to `_Atomic` objects through the ordinary operators.
//!
//! C11 gives an atomic object no special syntax: `x = v`, `x += v`, `x++`
//! and a plain read of `x` are the operations, and each is atomic. Reading
//! is a load and assigning a store with `memory_order_seq_cst` (6.2.6.1p9,
//! 7.17.3p6); a compound assignment and `++`/`--` are "a single evaluation"
//! of a read-modify-write with that order (6.5.16.2p3, 6.5.2.4p2).
//!
//! Until the type said atomic, lccc lowered all of these as it lowers a
//! `volatile` object's: `x++` on an `atomic_int` was `mov; lea; mov`, three
//! instructions between which another thread's update is lost, where GCC 16
//! and Clang 23 emit `lock add`. A reference count kept with `refs++` under
//! two threads came out short; so did every counter written the way the
//! standard says a counter may be written.
//!
//! Which lvalues are atomic is sema's verdict ([`Lowerer::lvalue_is_atomic`]);
//! what the lowering adds is the choice of operation:
//!
//! * a read is an `AtomicLoad`, an assignment an `AtomicStore`
//!   (`load_lvalue_typed` and `store_lvalue_typed`, by `LValue::atomic`);
//! * `+=`, `-=`, `&=`, `|=`, `^=`, `++` and `--` on an integer or a pointer
//!   are one `AtomicRmw`, which every backend has an instruction for;
//! * every other update -- `*=`, `/=`, `%=`, the shifts, any operator on a
//!   floating object or a `_Bool`, any operator whose other operand is
//!   floating, and the trapping forms of `-ftrapv` -- is a compare-exchange
//!   loop around the ordinary computation: read, compute, exchange if the
//!   object still holds what was read, else again. This is GCC's and
//!   Clang's lowering of the same operators.
//!
//! The objects covered are the ones an atomic instruction holds: integers
//! and pointers on every target (a 16-byte integer through libatomic's
//! `_16` entry points where the target has no instruction, as GCC does),
//! `float` and `double` on x86. A differently shaped atomic object (`long
//! double`, a complex number, a structure) keeps the volatile access it
//! had: GCC goes through libatomic's generic `__atomic_load`,
//! `__atomic_store` and `__atomic_compare_exchange` for those, and lccc
//! does not yet.

use super::definitions::{LValue, LValueKind};
use super::lower::Lowerer;
use crate::backend::Target;
use crate::common::types::{AddressSpace, IrType, SsoMode};
use crate::frontend::parser::ast::Expr;
use crate::ir::reexports::{
    AtomicOrdering, AtomicRmwOp, BlockId, Instruction, Operand, Terminator, Value,
};

/// A read-modify-write of an atomic object, between
/// [`Lowerer::begin_atomic_update`] and [`Lowerer::finish_atomic_update`].
pub(super) struct AtomicUpdate {
    ptr: Value,
    ty: IrType,
    /// The retry loop's head and exit; `None` when one `AtomicRmw` was the
    /// whole update.
    retry: Option<(BlockId, BlockId)>,
}

impl Lowerer {
    /// Whether the lvalue `expr` designates an `_Atomic` object: sema's
    /// verdict (`SemaResult::atomic_lvalues`), which rests on the qualifier
    /// records the lowering does not have. An expression sema never visited
    /// is not atomic here, and is accessed as before.
    pub(super) fn lvalue_is_atomic(&self, expr: &Expr) -> bool {
        !self.sema_atomic_lvalues.is_empty()
            && self
                .sema_atomic_lvalues
                .get(&expr.id())
                .is_some_and(|span| *span == expr.span())
    }

    /// Whether the target's atomic instructions hold an object of type `ty`.
    fn atomic_instruction_type(&self, ty: IrType) -> bool {
        match ty {
            IrType::I8
            | IrType::U8
            | IrType::I16
            | IrType::U16
            | IrType::I32
            | IrType::U32
            | IrType::I64
            | IrType::U64
            | IrType::Ptr => true,
            // The x86 atomic instructions move a float as its bits through
            // a general register; the other backends are not measured.
            IrType::F32 | IrType::F64 => matches!(self.target, Target::X86_64 | Target::I686),
            // Sixteen bytes: libatomic's `_16` entry points on x86-64 and
            // RISC-V (GCC's lowering, and `-latomic` at the link as with
            // GCC), exclusive pairs on AArch64. i686 has no such type.
            IrType::I128 | IrType::U128 => !matches!(self.target, Target::I686),
            _ => false,
        }
    }

    /// The address to hand an atomic instruction for the access `lv` of an
    /// object of type `ty`; `None` when the access is not an atomic one, or
    /// is one no atomic instruction expresses (a segment-relative object, a
    /// member in reverse storage order, a type the instructions do not hold).
    pub(super) fn atomic_target(&self, lv: &LValue, ty: IrType) -> Option<Value> {
        if !lv.atomic || !matches!(lv.sso, SsoMode::None) || !self.atomic_instruction_type(ty) {
            return None;
        }
        match lv.kind {
            LValueKind::Variable(ptr) | LValueKind::Address(ptr, AddressSpace::Default) => {
                Some(ptr)
            }
            _ => None,
        }
    }

    /// The value of an atomic scalar lvalue used as an rvalue (C11
    /// 6.3.2.1p2): one atomic load. `None` for anything else, which the
    /// ordinary lowering of the expression then reads.
    pub(super) fn try_lower_atomic_read(&mut self, expr: &Expr) -> Option<Operand> {
        if !self.lvalue_is_atomic(expr) {
            return None;
        }
        // An array or a structure is its address here, a function its own;
        // only an object that is read as a value has one to load.
        let ct = self.expr_ctype(expr);
        let scalar = ct.is_integer()
            || matches!(ct, crate::common::types::CType::Pointer(..))
            || (ct.is_floating() && !ct.is_complex());
        if !scalar {
            return None;
        }
        let ty = self.get_expr_type(expr);
        // (A bit-field is no object of its own; GCC refuses an atomic one.)
        let bit_field = match expr {
            Expr::MemberAccess(base, field, _) => {
                self.resolve_member_access_full(base, field).2.is_some()
            }
            Expr::PointerMemberAccess(base, field, _) => self
                .resolve_pointer_member_access_full(base, field)
                .2
                .is_some(),
            _ => false,
        };
        if !self.atomic_instruction_type(ty) || bit_field {
            return None;
        }
        // Decided before anything is emitted: an lvalue that turns out not
        // to be atomically addressable must not have been evaluated twice.
        if self.get_addr_space_of_struct_expr(expr) != AddressSpace::Default {
            return None;
        }
        let lv = self.lower_lvalue(expr)?;
        Some(self.load_lvalue_typed(&lv, ty))
    }

    pub(super) fn emit_atomic_load(&mut self, ptr: Value, ty: IrType) -> Operand {
        let dest = self.fresh_value();
        self.emit(Instruction::AtomicLoad {
            dest,
            ptr: Operand::Value(ptr),
            ty,
            ordering: AtomicOrdering::SeqCst,
        });
        Operand::Value(dest)
    }

    pub(super) fn emit_atomic_store(&mut self, ptr: Value, val: Operand, ty: IrType) {
        self.emit(Instruction::AtomicStore {
            ptr: Operand::Value(ptr),
            val,
            ty,
            ordering: AtomicOrdering::SeqCst,
        });
    }

    /// Begin a read-modify-write of the atomic object at `ptr` and return
    /// the value the update starts from.
    ///
    /// With `rmw`, the update is that one instruction and is complete when
    /// this returns: the value is the object's previous one, from which the
    /// caller computes what the expression yields, and
    /// [`Self::finish_atomic_update`] has nothing left to do.
    ///
    /// Without, a retry loop opens here. The value is a fresh atomic load;
    /// the caller computes the new one from it and `finish_atomic_update`
    /// exchanges it in if the object still holds what was loaded, or runs
    /// the loop again. Everything the caller emits in between is the loop
    /// body, so whatever must be evaluated once -- the address, the other
    /// operand -- has to be lowered before this call.
    pub(super) fn begin_atomic_update(
        &mut self,
        ptr: Value,
        ty: IrType,
        rmw: Option<(AtomicRmwOp, Operand)>,
    ) -> (AtomicUpdate, Operand) {
        if let Some((op, val)) = rmw {
            let old = self.fresh_value();
            self.emit(Instruction::AtomicRmw {
                dest: old,
                op,
                ptr: Operand::Value(ptr),
                val,
                ty,
                ordering: AtomicOrdering::SeqCst,
            });
            let update = AtomicUpdate {
                ptr,
                ty,
                retry: None,
            };
            return (update, Operand::Value(old));
        }
        let head = self.fresh_label();
        let done = self.fresh_label();
        self.terminate(Terminator::Branch(head));
        self.start_block(head);
        let old = self.emit_atomic_load(ptr, ty);
        let update = AtomicUpdate {
            ptr,
            ty,
            retry: Some((head, done)),
        };
        (update, old)
    }

    /// Complete the update `begin_atomic_update` opened: store `new` if the
    /// object still holds `old`, else take the loop again.
    ///
    /// The exchange compares representations, which is what makes it right
    /// for a floating object too: a NaN that compares unequal to itself, or
    /// a zero of the other sign, is the same object value or a different
    /// one exactly as the bits say.
    pub(super) fn finish_atomic_update(
        &mut self,
        update: AtomicUpdate,
        old: Operand,
        new: Operand,
    ) {
        let Some((head, done)) = update.retry else {
            return;
        };
        let exchanged = self.fresh_value();
        self.emit(Instruction::AtomicCmpxchg {
            dest: exchanged,
            ptr: Operand::Value(update.ptr),
            expected: old,
            desired: new,
            ty: update.ty,
            success_ordering: AtomicOrdering::SeqCst,
            failure_ordering: AtomicOrdering::SeqCst,
            returns_bool: true,
        });
        self.terminate(Terminator::CondBranch {
            cond: Operand::Value(exchanged),
            true_label: done,
            false_label: head,
        });
        self.start_block(done);
    }
}

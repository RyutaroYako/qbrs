//! `DynSelect`: `Select` with its join-topology-tracking `Scope` erased.

use std::marker::PhantomData;

use super::{Select, Selection, render_select_body};
use crate::dialect::Dialect;
use crate::expr::{ExprKind, Value};

/// This is the **one** genuinely unavoidable escape hatch in this design: a
/// single static type cannot mean "this table is joined" in one branch of
/// an `if`/`match` and "it isn't" in another, so conditionally varying
/// which tables get joined has no fully-static solution in any type
/// system, not just this one. `DynSelect` is deliberately narrow compared
/// to Drizzle's `.$dynamic()` (which discards chain-typing for the *entire*
/// query, including predicates/order-by that don't need it) or diesel's
/// `BoxableExpression` (type-erases a whole trait object per boxed
/// predicate): here, only the join skeleton is erased, every column
/// reference was already checked against a concrete `Scope` before
/// `.erase()` was ever called, and predicates/selection stay in the same
/// closed, non-generic `ExprKind`/`Value` representation used everywhere
/// else — no `Box<dyn _>` anywhere.
///
/// Once erased, `DynSelect` intentionally offers no further
/// `.filter()`/`.join()`/etc. — composition happens *before* erasure, on
/// the concrete `Select`; `DynSelect` exists only to let two
/// already-fully-built branches with different join topology unify into
/// one value for the purpose of choosing between them at runtime and then
/// rendering/executing. Widening its API to support further post-erasure
/// composition would start reproducing Diesel's/Drizzle's much broader
/// escape hatches instead of staying narrow.
pub struct DynSelect<D, Output> {
    ctes: Vec<super::CteDef>,
    from_table: &'static str,
    joins: Vec<super::JoinClause>,
    wheres: Vec<ExprKind>,
    order_by: Vec<(ExprKind, super::SortDir)>,
    group_by: Vec<ExprKind>,
    having: Vec<ExprKind>,
    limit: Option<i64>,
    offset: Option<i64>,
    selection_exprs: Vec<ExprKind>,
    _marker: PhantomData<fn() -> (D, Output)>,
}

impl<D, Scope, Sel> Select<D, Scope, Sel> {
    /// Erases `Scope`. `Sel::exprs()` is called *now*, while `Scope`/`Idx`
    /// are still known (they're required to resolve which `Selection` impl
    /// applies at all) — only the already-rendered `Vec<ExprKind>` and the
    /// plain-Rust `Output` type carry forward, which is all `DynSelect`
    /// needs, and neither depends on `Scope` any more once computed.
    pub fn erase<Idx>(self) -> DynSelect<D, Sel::Output>
    where
        Sel: Selection<Scope, Idx>,
    {
        DynSelect {
            ctes: self.ctes,
            from_table: self.from_table,
            joins: self.joins,
            wheres: self.wheres,
            order_by: self.order_by,
            group_by: self.group_by,
            having: self.having,
            limit: self.limit,
            offset: self.offset,
            selection_exprs: self.selection.exprs(),
            _marker: PhantomData,
        }
    }
}

impl<D: Dialect, Output> DynSelect<D, Output> {
    pub fn to_sql(&self) -> (String, Vec<Value>) {
        render_select_body::<D>(
            &self.ctes,
            self.from_table,
            &self.joins,
            &self.selection_exprs,
            &self.wheres,
            &self.group_by,
            &self.having,
            &self.order_by,
            self.limit,
            self.offset,
        )
    }
}

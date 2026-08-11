//! `UNION`/`UNION ALL`/`INTERSECT`/`EXCEPT` between two `SELECT`s that may
//! have entirely different `Scope`s (different tables, different JOINs) —
//! the only thing that must line up is their *output shape*. Rather than
//! inventing a separate `SameShape<A, B>` trait (as the design plan
//! originally sketched), this reuses `Selection::Output` associated-type
//! equality directly: `SelB: Selection<ScopeB, IdxB, Output = Sel::Output>`
//! already means "decodes to the exact same Rust tuple", which is exactly
//! what a SQL set operation requires (same column count, compatible types)
//! — no new trait needed.
//!
//! Each branch is rendered independently via `RawEmbed<D>` (see its doc
//! comment) into `?`-placeholder text, then spliced together and renumbered
//! into the outer dialect's placeholder style at `.to_sql()` time via
//! `render::splice_raw` — the same mechanism `Select::exists`/`not_exists`
//! use to embed a subquery, generalized to top-level branches joined by a
//! set operator instead of by `EXISTS (..)`.

use std::marker::PhantomData;

use super::{Select, Selection, SortDir};
use crate::dialect::{Dialect, RawEmbed};
use crate::expr::Value;
use crate::render::splice_raw;

enum SetOpKind {
    Union,
    UnionAll,
    Intersect,
    Except,
}

impl SetOpKind {
    fn keyword(&self) -> &'static str {
        match self {
            SetOpKind::Union => " UNION ",
            SetOpKind::UnionAll => " UNION ALL ",
            SetOpKind::Intersect => " INTERSECT ",
            SetOpKind::Except => " EXCEPT ",
        }
    }
}

/// A chain of `SELECT`s combined by set operators, all decoding to the same
/// `Output` type. `ORDER BY` here is necessarily by **ordinal position**
/// (`ORDER BY 1`, 1-indexed) rather than a typed column — the branches can
/// have entirely different `Scope`s, so there is no single scope left to
/// check a column reference against once they're combined; ordinal position
/// is the only reference SQL itself allows in this position.
pub struct SetOp<D, Output> {
    first: (String, Vec<Value>),
    rest: Vec<(SetOpKind, String, Vec<Value>)>,
    order_by: Vec<(u32, SortDir)>,
    limit: Option<i64>,
    offset: Option<i64>,
    _marker: PhantomData<fn() -> (D, Output)>,
}

impl<D: Dialect, Output> SetOp<D, Output> {
    fn new(first: (String, Vec<Value>)) -> Self {
        SetOp {
            first,
            rest: Vec::new(),
            order_by: Vec::new(),
            limit: None,
            offset: None,
            _marker: PhantomData,
        }
    }

    fn push(mut self, kind: SetOpKind, branch: (String, Vec<Value>)) -> Self {
        self.rest.push((kind, branch.0, branch.1));
        self
    }

    /// Appends another branch via `UNION` (duplicates across branches are
    /// removed, same as plain SQL `UNION`).
    pub fn union<ScopeB, SelB, IdxB>(self, other: &Select<D, ScopeB, SelB>) -> Self
    where
        SelB: Selection<ScopeB, IdxB, Output = Output>,
    {
        self.push(SetOpKind::Union, other.render_as::<RawEmbed<D>, IdxB>())
    }

    /// Appends another branch via `UNION ALL` (no deduplication — cheaper
    /// than `UNION` when the branches are already known disjoint, or when
    /// duplicates are meaningful).
    pub fn union_all<ScopeB, SelB, IdxB>(self, other: &Select<D, ScopeB, SelB>) -> Self
    where
        SelB: Selection<ScopeB, IdxB, Output = Output>,
    {
        self.push(SetOpKind::UnionAll, other.render_as::<RawEmbed<D>, IdxB>())
    }

    /// Appends another branch via `INTERSECT` (rows present in both).
    pub fn intersect<ScopeB, SelB, IdxB>(self, other: &Select<D, ScopeB, SelB>) -> Self
    where
        SelB: Selection<ScopeB, IdxB, Output = Output>,
    {
        self.push(SetOpKind::Intersect, other.render_as::<RawEmbed<D>, IdxB>())
    }

    /// Appends another branch via `EXCEPT` (rows in the accumulated result
    /// so far, minus rows in `other`).
    pub fn except<ScopeB, SelB, IdxB>(self, other: &Select<D, ScopeB, SelB>) -> Self
    where
        SelB: Selection<ScopeB, IdxB, Output = Output>,
    {
        self.push(SetOpKind::Except, other.render_as::<RawEmbed<D>, IdxB>())
    }

    /// Orders the combined result by the `position`th (1-indexed) selected
    /// column — see this struct's doc comment for why ordinal position,
    /// not a typed column, is the only option here. Callable multiple
    /// times like `Select::order_by`, each call appending a sort key.
    pub fn order_by(mut self, position: u32, dir: SortDir) -> Self {
        self.order_by.push((position, dir));
        self
    }

    pub fn limit(mut self, n: i64) -> Self {
        self.limit = Some(n);
        self
    }

    pub fn offset(mut self, n: i64) -> Self {
        self.offset = Some(n);
        self
    }

    pub fn to_sql(&self) -> (String, Vec<Value>) {
        let mut sql = String::new();
        let mut params = Vec::new();

        sql.push('(');
        splice_raw::<D>(&self.first.0, &self.first.1, &mut sql, &mut params);
        sql.push(')');

        for (kind, text, branch_params) in &self.rest {
            sql.push_str(kind.keyword());
            sql.push('(');
            splice_raw::<D>(text, branch_params, &mut sql, &mut params);
            sql.push(')');
        }

        if !self.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            for (i, (position, dir)) in self.order_by.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                sql.push_str(&position.to_string());
                sql.push_str(match dir {
                    SortDir::Asc => " ASC",
                    SortDir::Desc => " DESC",
                });
            }
        }
        if let Some(l) = self.limit {
            sql.push_str(" LIMIT ");
            sql.push_str(&l.to_string());
        }
        if let Some(o) = self.offset {
            sql.push_str(" OFFSET ");
            sql.push_str(&o.to_string());
        }

        (sql, params)
    }
}

impl<D: Dialect, Scope, Sel> Select<D, Scope, Sel> {
    /// Starts a `UNION` chain — see `SetOp`'s doc comment for why the two
    /// branches only need matching `Selection::Output`, not matching
    /// `Scope`.
    pub fn union<ScopeB, SelB, IdxA, IdxB>(
        &self,
        other: &Select<D, ScopeB, SelB>,
    ) -> SetOp<D, Sel::Output>
    where
        Sel: Selection<Scope, IdxA>,
        SelB: Selection<ScopeB, IdxB, Output = Sel::Output>,
    {
        SetOp::new(self.render_as::<RawEmbed<D>, IdxA>()).union(other)
    }

    pub fn union_all<ScopeB, SelB, IdxA, IdxB>(
        &self,
        other: &Select<D, ScopeB, SelB>,
    ) -> SetOp<D, Sel::Output>
    where
        Sel: Selection<Scope, IdxA>,
        SelB: Selection<ScopeB, IdxB, Output = Sel::Output>,
    {
        SetOp::new(self.render_as::<RawEmbed<D>, IdxA>()).union_all(other)
    }

    pub fn intersect<ScopeB, SelB, IdxA, IdxB>(
        &self,
        other: &Select<D, ScopeB, SelB>,
    ) -> SetOp<D, Sel::Output>
    where
        Sel: Selection<Scope, IdxA>,
        SelB: Selection<ScopeB, IdxB, Output = Sel::Output>,
    {
        SetOp::new(self.render_as::<RawEmbed<D>, IdxA>()).intersect(other)
    }

    pub fn except<ScopeB, SelB, IdxA, IdxB>(
        &self,
        other: &Select<D, ScopeB, SelB>,
    ) -> SetOp<D, Sel::Output>
    where
        Sel: Selection<Scope, IdxA>,
        SelB: Selection<ScopeB, IdxB, Output = Sel::Output>,
    {
        SetOp::new(self.render_as::<RawEmbed<D>, IdxA>()).except(other)
    }
}

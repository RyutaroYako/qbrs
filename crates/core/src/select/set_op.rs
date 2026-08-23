//! `UNION`/`UNION ALL`/`INTERSECT`/`EXCEPT` between two `SELECT`s that may
//! have entirely different `Scope`s (different tables, different JOINs) —
//! the only thing that must line up is their *output shape*: `row::SameShape`
//! requires the same column names, in the same order, decoding to the same
//! types. Names as well as types, because the combined result is read by key
//! — a branch whose columns merely happen to be type-compatible would
//! otherwise splice in transposed. Keys from different tables still match,
//! since the comparison is on names, and SQL itself takes a `UNION`'s column
//! names from the first branch.

use std::marker::PhantomData;

use super::{Select, Selection, SortDir};
use crate::dialect::Dialect;
use crate::expr::Value;
use crate::render::{Fragment, QuerySink, Sink};
use crate::row::SameShape;

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

/// A chain of `SELECT`s combined by set operators, all decoding to the first
/// branch's `Output` — which is also where SQL itself takes the combined
/// result's column names from. `ORDER BY` here is necessarily by **ordinal position**
/// (`ORDER BY 1`, 1-indexed) rather than a typed column — the branches can
/// have entirely different `Scope`s, so there is no single scope left to
/// check a column reference against once they're combined; ordinal position
/// is the only reference SQL itself allows in this position.
pub struct SetOp<D, Output> {
    first: Fragment,
    rest: Vec<(SetOpKind, Fragment)>,
    order_by: Vec<(u32, SortDir)>,
    limit: Option<super::RowCount>,
    offset: Option<super::RowCount>,
    _marker: PhantomData<fn() -> (D, Output)>,
}

impl<D: Dialect, L> SetOp<D, crate::row::Row<L>> {
    /// `ORDER BY` naming the column instead of counting to it: the position
    /// is `row::Field`'s index, which the row already carries. A column the
    /// combined result doesn't select is a compile error, which is the whole
    /// reason no `ORDER BY <n>` is spellable here. Callable multiple times
    /// like `Select::order_by`, each call appending a key.
    pub fn order_by_column<K, Idx>(self, _key: K, dir: SortDir) -> Self
    where
        K: crate::row::LookupKey,
        L: crate::row::Field<K::Key, Idx>,
        Idx: crate::scope::Position,
    {
        self.order_by_ordinal(<Idx as crate::scope::Position>::POSITION, dir)
    }
}

/// A set operation whose branches select one un-tupled column: its output
/// is a bare value, so there is one position and nothing to name.
impl<D: Dialect, V: crate::select::SingleColumn> SetOp<D, V> {
    pub fn order_by(self, dir: SortDir) -> Self {
        self.order_by_ordinal(1, dir)
    }
}

impl<D: Dialect, Output> SetOp<D, Output> {
    fn new(first: Fragment) -> Self {
        SetOp {
            first,
            rest: Vec::new(),
            order_by: Vec::new(),
            limit: None,
            offset: None,
            _marker: PhantomData,
        }
    }

    fn push(mut self, kind: SetOpKind, branch: Fragment) -> Self {
        self.rest.push((kind, branch));
        self
    }

    /// Appends another branch via `UNION` (duplicates across branches are
    /// removed, same as plain SQL `UNION`).
    pub fn union<ScopeB, SelB, IdxB>(self, other: &Select<D, ScopeB, SelB>) -> Self
    where
        SelB: Selection<ScopeB, IdxB>,
        SelB::Output: SameShape<Output>,
    {
        self.push(SetOpKind::Union, other.fragment::<IdxB>())
    }

    /// Appends another branch via `UNION ALL` (no deduplication — cheaper
    /// than `UNION` when the branches are already known disjoint, or when
    /// duplicates are meaningful).
    pub fn union_all<ScopeB, SelB, IdxB>(self, other: &Select<D, ScopeB, SelB>) -> Self
    where
        SelB: Selection<ScopeB, IdxB>,
        SelB::Output: SameShape<Output>,
    {
        self.push(SetOpKind::UnionAll, other.fragment::<IdxB>())
    }

    /// Appends another branch via `INTERSECT` (rows present in both).
    pub fn intersect<ScopeB, SelB, IdxB>(self, other: &Select<D, ScopeB, SelB>) -> Self
    where
        SelB: Selection<ScopeB, IdxB>,
        SelB::Output: SameShape<Output>,
    {
        self.push(SetOpKind::Intersect, other.fragment::<IdxB>())
    }

    /// Appends another branch via `EXCEPT` (rows in the accumulated result
    /// so far, minus rows in `other`).
    pub fn except<ScopeB, SelB, IdxB>(self, other: &Select<D, ScopeB, SelB>) -> Self
    where
        SelB: Selection<ScopeB, IdxB>,
        SelB::Output: SameShape<Output>,
    {
        self.push(SetOpKind::Except, other.fragment::<IdxB>())
    }

    fn order_by_ordinal(mut self, position: u32, dir: SortDir) -> Self {
        self.order_by.push((position, dir));
        self
    }

    pub fn limit(mut self, n: impl super::IntoRowCount) -> Self {
        self.limit = Some(n.into_row_count());
        self
    }

    pub fn offset(mut self, n: impl super::IntoRowCount) -> Self {
        self.offset = Some(n.into_row_count());
        self
    }

    /// How many rows the combination returns, its own `ORDER BY`/paging
    /// dropped. The branches keep theirs: a `UNION` of two `LIMIT`ed queries
    /// is a different set from a `UNION` of the whole ones.
    pub fn count_sql(&self, _dialect: D) -> (String, Vec<Value>) {
        let mut sink = QuerySink::<D>::new();
        crate::render::render_count_wrapped::<D>(&mut sink, |sink| self.render_branches(sink));
        sink.finish()
    }

    pub fn to_sql(&self, _dialect: D) -> (String, Vec<Value>) {
        let mut sink = QuerySink::<D>::new();
        self.render_branches(&mut sink);
        self.render_ordering(&mut sink);
        sink.finish()
    }

    /// The set operation itself, without the ordering and paging applied to
    /// its result — which is what a count of it must leave out.
    fn render_branches(&self, sink: &mut QuerySink<D>) {
        let branch = |sink: &mut QuerySink<D>, sql: &Fragment| {
            // A branch has to be shut off from the operator beside it:
            // otherwise its `ORDER BY`/`LIMIT`, or its `WITH`, reads as the
            // whole compound's and the statement doesn't parse. Where a
            // dialect has no parentheses for that (SQLite), a derived table
            // says the same thing — and saying it unconditionally is what
            // keeps a clause added later from slipping through.
            if D::PARENTHESIZED_SET_OP_BRANCHES {
                sink.ch('(');
                sql.splice_into(sink);
                sink.ch(')');
            } else {
                sink.text("SELECT * FROM (");
                sql.splice_into(sink);
                sink.ch(')');
            }
        };

        branch(sink, &self.first);
        for (kind, part) in &self.rest {
            sink.text(kind.keyword());
            branch(sink, part);
        }
    }

    fn render_ordering(&self, sink: &mut QuerySink<D>) {
        if !self.order_by.is_empty() {
            sink.text(" ORDER BY ");
            for (i, (position, dir)) in self.order_by.iter().enumerate() {
                if i > 0 {
                    sink.text(", ");
                }
                sink.text(&position.to_string());
                sink.text(crate::render::dir_keyword(*dir));
            }
        }
        crate::select::render_limit_offset::<D>(sink, self.limit.as_ref(), self.offset.as_ref());
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
        SelB: Selection<ScopeB, IdxB>,
        SelB::Output: SameShape<Sel::Output>,
    {
        SetOp::new(self.fragment::<IdxA>()).union(other)
    }

    pub fn union_all<ScopeB, SelB, IdxA, IdxB>(
        &self,
        other: &Select<D, ScopeB, SelB>,
    ) -> SetOp<D, Sel::Output>
    where
        Sel: Selection<Scope, IdxA>,
        SelB: Selection<ScopeB, IdxB>,
        SelB::Output: SameShape<Sel::Output>,
    {
        SetOp::new(self.fragment::<IdxA>()).union_all(other)
    }

    pub fn intersect<ScopeB, SelB, IdxA, IdxB>(
        &self,
        other: &Select<D, ScopeB, SelB>,
    ) -> SetOp<D, Sel::Output>
    where
        Sel: Selection<Scope, IdxA>,
        SelB: Selection<ScopeB, IdxB>,
        SelB::Output: SameShape<Sel::Output>,
    {
        SetOp::new(self.fragment::<IdxA>()).intersect(other)
    }

    pub fn except<ScopeB, SelB, IdxA, IdxB>(
        &self,
        other: &Select<D, ScopeB, SelB>,
    ) -> SetOp<D, Sel::Output>
    where
        Sel: Selection<Scope, IdxA>,
        SelB: Selection<ScopeB, IdxB>,
        SelB::Output: SameShape<Sel::Output>,
    {
        SetOp::new(self.fragment::<IdxA>()).except(other)
    }
}

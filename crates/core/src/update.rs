//! `UPDATE .. SET .. WHERE ..`.

use std::marker::PhantomData;

use crate::dialect::Dialect;
use crate::expr::{AssignsTo, Column, ColumnKey, ExprKind, IntoExpr, Value, Writable};
use crate::render::{Sink, render_and_list, render_expr, render_ident};
use crate::scope::{BaseTable, Superset, Table};
use crate::select::{Condition, Predicate};
use crate::statement::{Statement, WrittenTable};

/// Implemented by the `#[derive(Table)]`-generated `*Update` struct: every
/// field is optional (untouched vs. touched), and doubly-optional for
/// nullable columns (untouched vs. explicit NULL vs. explicit value).
/// `sets()` returns only the touched `(column, value)` pairs.
pub trait UpdateRow: private::Sealed {
    type Table: Table;
    fn sets(self) -> Vec<(&'static str, Value)>;
}

mod private {
    /// `sets()` names columns of `Table` by string; `#[derive(Table)]` is
    /// what guarantees they exist, so it is the only thing that can produce
    /// an `UpdateRow`.
    pub trait Sealed {}
}

#[doc(hidden)]
pub use private::Sealed as UpdateRowSealed;

/// A `SET` list that is known non-empty, which is the only kind that has a
/// SQL form. Every `*Update` derives `Default`, and that value (what a
/// PATCH handler holds when the request changed nothing) has no assignments
/// at all. The check therefore belongs where such a value enters a
/// statement rather than at rendering time.
pub struct Assignments<T> {
    sets: Vec<(&'static str, ExprKind)>,
    _marker: PhantomData<fn() -> T>,
}

// Hand-written for the reason `Expr`'s are: a derive would ask the phantom
// table marker to be `Clone`/`Debug`, and a schema's marker is a bare unit
// struct, so the derived impls would apply to no table at all.
impl<T> Clone for Assignments<T> {
    fn clone(&self) -> Self {
        Assignments {
            sets: self.sets.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T> std::fmt::Debug for Assignments<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Assignments")
            .field("sets", &self.sets)
            .finish()
    }
}

impl<T: Table> Assignments<T> {
    /// `column = <expression>`. This is for the assignments a value can't
    /// say: `updated_at = now()`, `version = version + 1`. The expression is
    /// checked against the table being written to, exactly as a `WHERE`
    /// condition is.
    pub fn set_to<C, V, Idxs>(_column: Column<C>, value: V) -> Self
    where
        C: ColumnKey<Table = T> + Writable,
        V: IntoExpr,
        V::Sql: AssignsTo<C::Sql>,
        WrittenTable<T>: Superset<V::Req, Idxs>,
    {
        Assignments {
            sets: vec![(<C as crate::row::Named>::NAME, value.into_expr().kind)],
            _marker: PhantomData,
        }
    }

    /// One more of them, so a statement can assign several expressions.
    pub fn and_set_to<C, V, Idxs>(mut self, _column: Column<C>, value: V) -> Self
    where
        C: ColumnKey<Table = T> + Writable,
        V: IntoExpr,
        V::Sql: AssignsTo<C::Sql>,
        WrittenTable<T>: Superset<V::Req, Idxs>,
    {
        push_set(
            &mut self.sets,
            <C as crate::row::Named>::NAME,
            value.into_expr().kind,
        );
        self
    }
}

/// A column assigned twice is not a statement any database accepts, and
/// layering a computed assignment over a request's is exactly when it
/// happens, so the later one replaces the earlier. Shared with the
/// `ON CONFLICT DO UPDATE` list, which is built the same way.
pub(crate) fn push_set(
    sets: &mut Vec<(&'static str, ExprKind)>,
    name: &'static str,
    value: ExprKind,
) {
    sets.retain(|(col, _)| *col != name);
    sets.push((name, value));
}

impl<T> Assignments<T> {
    /// The list itself, for the one other statement that renders a `SET`.
    pub(crate) fn into_sets(self) -> Vec<(&'static str, ExprKind)> {
        self.sets
    }

    /// `col = $n, col = $n`: the one renderer for a `SET` list, shared by
    /// `UPDATE` and `ON CONFLICT DO UPDATE`.
    pub(crate) fn render_into<D: Dialect>(&self, sink: &mut dyn Sink) {
        for (i, (col, value)) in self.sets.iter().enumerate() {
            if i > 0 {
                sink.text(", ");
            }
            render_ident::<D>(sink, col);
            sink.text(" = ");
            render_expr::<D>(value, sink);
        }
    }

    /// The `SET` list a request struct describes. This is the one fallible
    /// step in building a statement, since a `*Update` whose every field is
    /// untouched describes no assignment. Mixed with computed ones as
    /// `Assignments::from_row(patch)?.and_set_to(col, expr)`.
    pub fn from_row<R: UpdateRow<Table = T>>(row: R) -> Result<Self, NothingToSet> {
        let mut sets: Vec<(&'static str, ExprKind)> = Vec::new();
        for (col, value) in row.sets() {
            push_set(&mut sets, col, ExprKind::Value(value));
        }
        if sets.is_empty() {
            return Err(NothingToSet);
        }
        Ok(Assignments {
            sets,
            _marker: PhantomData,
        })
    }
}

/// Every field of an `*Update` was untouched, so there is nothing to
/// assign. Returned rather than panicked because request-shaped data
/// produces it: the caller decides whether that is a no-op or an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NothingToSet;

impl std::fmt::Display for NothingToSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an UPDATE must set at least one column, but every field of this `*Update` is untouched")
    }
}
impl std::error::Error for NothingToSet {}

pub struct UpdateSeed<D, T> {
    _marker: PhantomData<fn() -> (D, T)>,
}

pub fn update<D, T: BaseTable>(_table: T) -> UpdateSeed<D, T> {
    UpdateSeed {
        _marker: PhantomData,
    }
}

impl<D, T: Table> UpdateSeed<D, T> {
    /// A correlated subquery over the table this statement will write.
    /// It is available before the `SET` list, since the scope it correlates
    /// against is the table, not the assignments.
    pub fn correlated<S, InnerSel>(
        &self,
        source: S,
        selection: InnerSel,
    ) -> crate::select::Select<
        D,
        crate::scope::Cons<
            crate::scope::TableSlot<S::Table, crate::scope::NotNull>,
            WrittenTable<T>,
        >,
        InnerSel,
        WrittenTable<T>,
    >
    where
        S: crate::select::JoinSource<D>,
    {
        crate::select::correlated_with(source, selection)
    }

    /// `SET column = <expression>` as the statement's first assignment.
    /// Infallible, since one assignment is one assignment. `.set_to(..)`
    /// again for more.
    pub fn set_to<C, V, Idxs>(self, column: Column<C>, value: V) -> Update<D, T>
    where
        C: ColumnKey<Table = T> + Writable,
        V: IntoExpr,
        V::Sql: AssignsTo<C::Sql>,
        WrittenTable<T>: Superset<V::Req, Idxs>,
    {
        Update {
            sets: Assignments::set_to(column, value),
            wheres: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// The statement's `SET` list. Infallible: an `Assignments` holds at
    /// least one assignment by construction, and the empty case an
    /// `*Update` can be lives in `Assignments::from_row`, which is where
    /// the `?` goes.
    pub fn set(self, sets: Assignments<T>) -> Update<D, T> {
        Update {
            sets,
            wheres: Vec::new(),
            _marker: PhantomData,
        }
    }
}

fn render_set_clause<D: Dialect, T: Table>(
    sink: &mut dyn Sink,
    sets: &Assignments<T>,
    wheres: &[ExprKind],
) {
    sink.text("UPDATE ");
    render_ident::<D>(sink, T::NAME);
    sink.text(" SET ");
    sets.render_into::<D>(sink);
    render_and_list::<D>(sink, " WHERE ", wheres);
}

pub struct Update<D, T: Table> {
    sets: Assignments<T>,
    wheres: Vec<ExprKind>,
    _marker: PhantomData<fn() -> (D, T)>,
}

impl<D, T: Table> Update<D, T> {
    /// A correlated subquery over the table this statement writes. It is
    /// the same `EXISTS` a `SELECT` builds with `Select::correlated`,
    /// against the one-table scope a write statement has.
    pub fn correlated<S, InnerSel>(
        &self,
        source: S,
        selection: InnerSel,
    ) -> crate::select::Select<
        D,
        crate::scope::Cons<
            crate::scope::TableSlot<S::Table, crate::scope::NotNull>,
            WrittenTable<T>,
        >,
        InnerSel,
        WrittenTable<T>,
    >
    where
        S: crate::select::JoinSource<D>,
    {
        crate::select::correlated_with(source, selection)
    }

    pub fn filter<C: Condition<D, WrittenTable<T>, Idxs>, Idxs>(mut self, cond: C) -> Self {
        self.wheres.push(cond.into_predicate().into_kind());
        self
    }

    /// AND-folds a runtime-length collection of discharged conditions, the
    /// same way `Select::filter_all` does. A `PATCH` narrows its rows by
    /// however many criteria the request carried.
    pub fn filter_all(
        mut self,
        conds: impl IntoIterator<Item = Predicate<D, WrittenTable<T>>>,
    ) -> Self {
        self.wheres
            .extend(conds.into_iter().map(Predicate::into_kind));
        self
    }
}

impl<D, T: Table> Update<D, T> {
    /// One more assignment, appended to whatever `.set(..)` already
    /// assigned. See `Assignments::set_to`.
    pub fn set_to<C, V, Idxs>(mut self, column: Column<C>, value: V) -> Self
    where
        C: ColumnKey<Table = T> + Writable,
        V: IntoExpr,
        V::Sql: AssignsTo<C::Sql>,
        WrittenTable<T>: Superset<V::Req, Idxs>,
    {
        self.sets = self.sets.and_set_to(column, value);
        self
    }
}

impl<D: Dialect, T: Table> crate::statement::private::Sealed for Update<D, T> {}

impl<D: Dialect, T: Table> Statement for Update<D, T> {
    type Dialect = D;
    type Table = T;
    fn render_into(&self, sink: &mut dyn Sink) {
        render_set_clause::<D, T>(sink, &self.sets, &self.wheres);
    }
}

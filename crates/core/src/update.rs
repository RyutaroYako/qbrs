//! `UPDATE .. SET .. WHERE ..`.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsReturning};
use crate::expr::{Bool, Expr, ExprKind, Value};
use crate::render::{QuerySink, Sink, render_and_list, render_ident};
use crate::scope::{BaseTable, Cons, Nil, NotNull, Superset, Table, TableSlot};
use crate::select::{Predicate, Selection};
use crate::statement::{Returning, Statement, WrittenTable};

/// Implemented by the `#[derive(Table)]`-generated `*Update` struct: every
/// field is optional (untouched vs. touched), and doubly-optional for
/// nullable columns (untouched vs. explicit NULL vs. explicit value).
/// `sets()` returns only the touched `(column, value)` pairs.
pub trait UpdateRow {
    type Table: Table;
    fn sets(self) -> Vec<(&'static str, Value)>;
}

/// A `SET` list that is known non-empty, which is the only kind that has a
/// SQL form. Every `*Update` derives `Default`, and that value — what a
/// PATCH handler holds when the request changed nothing — has no
/// assignments at all, so the check belongs where such a value enters a
/// statement rather than at rendering time.
pub struct Assignments {
    sets: Vec<(&'static str, Value)>,
}

impl Assignments {
    /// `col = $n, col = $n` — the one renderer for a `SET` list, shared by
    /// `UPDATE` and `ON CONFLICT DO UPDATE`.
    pub(crate) fn render_into<D: Dialect>(&self, sink: &mut dyn Sink) {
        for (i, (col, val)) in self.sets.iter().enumerate() {
            if i > 0 {
                sink.text(", ");
            }
            render_ident::<D>(sink, col);
            sink.text(" = ");
            sink.bind(val);
        }
    }

    pub(crate) fn new<R: UpdateRow>(row: R) -> Result<Self, NothingToSet> {
        let sets = row.sets();
        if sets.is_empty() {
            return Err(NothingToSet);
        }
        Ok(Assignments { sets })
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
    pub fn set<R: UpdateRow<Table = T>>(self, row: R) -> Result<Update<D, T>, NothingToSet> {
        Ok(Update {
            sets: Assignments::new(row)?,
            wheres: Vec::new(),
            _marker: PhantomData,
        })
    }
}

fn render_set_clause<D: Dialect, T: Table>(
    sets: &Assignments,
    wheres: &[ExprKind],
) -> QuerySink<D> {
    let mut sink = QuerySink::<D>::new();
    sink.text("UPDATE ");
    render_ident::<D>(&mut sink, T::NAME);
    sink.text(" SET ");
    sets.render_into::<D>(&mut sink);

    render_and_list::<D>(&mut sink, " WHERE ", wheres);

    sink
}

pub struct Update<D, T: Table> {
    sets: Assignments,
    wheres: Vec<ExprKind>,
    _marker: PhantomData<fn() -> (D, T)>,
}

impl<D, T: Table> Update<D, T> {
    /// Only columns of the table being updated are ever in scope for the
    /// `WHERE` clause here, so the `Superset` check is against a
    /// single-table scope rather than a full query `Scope`.
    pub fn filter<Req, Idxs>(mut self, cond: Expr<Req, Bool>) -> Self
    where
        Cons<TableSlot<T, NotNull>, Nil>: Superset<Req, Idxs>,
    {
        self.wheres.push(cond.kind);
        self
    }

    /// AND-folds a runtime-length collection of discharged conditions, the
    /// same way `Select::filter_all` does — a `PATCH` narrows its rows by
    /// however many criteria the request carried.
    pub fn filter_all(
        mut self,
        conds: impl IntoIterator<Item = Predicate<Cons<TableSlot<T, NotNull>, Nil>>>,
    ) -> Self {
        self.wheres
            .extend(conds.into_iter().map(Predicate::into_kind));
        self
    }
}

impl<D: Dialect, T: Table> crate::statement::StatementSealed for Update<D, T> {}

impl<D: Dialect, T: Table> Statement for Update<D, T> {
    type Dialect = D;
    type Table = T;
    fn render(&self) -> QuerySink<D> {
        render_set_clause::<D, T>(&self.sets, &self.wheres)
    }
}

impl<D: SupportsReturning, T: Table> Update<D, T> {
    /// A distinct type rather than `Self` with a flag set, for the reason
    /// `delete::Delete::returning` gives.
    pub fn returning<Sel, Idx>(self, sel: Sel) -> Returning<Self, Sel>
    where
        Sel: Selection<WrittenTable<T>, Idx>,
    {
        Returning {
            returning: sel.items(),
            statement: self,
            _marker: PhantomData,
        }
    }
}

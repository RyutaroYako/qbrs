//! `UPDATE .. SET .. WHERE ..`.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsReturning};
use crate::expr::{Bool, Expr, ExprKind, Value};
use crate::render::{QuerySink, SelectItem, Sink, render_expr, render_ident, render_select_list};
use crate::scope::{BaseTable, Cons, Nil, NotNull, Superset, Table, TableSlot};
use crate::select::Selection;

/// Implemented by the `#[derive(Table)]`-generated `*Update` struct: every
/// field is optional (untouched vs. touched), and doubly-optional for
/// nullable columns (untouched vs. explicit NULL vs. explicit value).
/// `sets()` returns only the touched `(column, value)` pairs.
pub trait UpdateRow {
    type Table: Table;
    fn sets(self) -> Vec<(&'static str, Value)>;
}

pub struct UpdateSeed<D, T> {
    _marker: PhantomData<fn() -> (D, T)>,
}

pub fn update<D, T: BaseTable>(_table: T) -> UpdateSeed<D, T> {
    UpdateSeed {
        _marker: PhantomData,
    }
}

impl<D, T: Table> UpdateSeed<D, T> {
    pub fn set<R: UpdateRow<Table = T>>(self, row: R) -> Update<D, T> {
        Update {
            sets: row.sets(),
            wheres: Vec::new(),
            _marker: PhantomData,
        }
    }
}

fn render_set_clause<D: Dialect, T: Table>(
    sets: &[(&'static str, Value)],
    wheres: &[ExprKind],
) -> QuerySink<D> {
    // An `UPDATE` with nothing set has no SQL form, and `*Update`'s derived
    // `Default` is exactly that shape — the state a PATCH handler holds when
    // the request changed nothing.
    assert!(
        !sets.is_empty(),
        "an UPDATE must set at least one column; every field of this `*Update` is untouched"
    );

    let mut sink = QuerySink::<D>::new();
    sink.text("UPDATE ");
    render_ident::<D>(&mut sink, T::NAME);
    sink.text(" SET ");
    for (i, (col, val)) in sets.iter().enumerate() {
        if i > 0 {
            sink.text(", ");
        }
        render_ident::<D>(&mut sink, col);
        sink.text(" = ");
        sink.bind(val);
    }

    if !wheres.is_empty() {
        sink.text(" WHERE ");
        for (i, w) in wheres.iter().enumerate() {
            if i > 0 {
                sink.text(" AND ");
            }
            render_expr::<D>(w, &mut sink);
        }
    }

    sink
}

pub struct Update<D, T: Table> {
    sets: Vec<(&'static str, Value)>,
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
}

impl<D: Dialect, T: Table> Update<D, T> {
    pub fn to_sql(&self) -> (String, Vec<Value>) {
        render_set_clause::<D, T>(&self.sets, &self.wheres).finish()
    }
}

impl<D: SupportsReturning, T: Table> Update<D, T> {
    /// A distinct type rather than `Self` with a flag set, for the reason
    /// `insert::Insert::returning` gives.
    pub fn returning<Sel, Idx>(self, sel: Sel) -> UpdateReturning<D, T, Sel>
    where
        Sel: Selection<Cons<TableSlot<T, NotNull>, Nil>, Idx>,
    {
        UpdateReturning {
            sets: self.sets,
            wheres: self.wheres,
            returning: sel.items(),
            _marker: PhantomData,
        }
    }
}

pub struct UpdateReturning<D, T: Table, Sel> {
    sets: Vec<(&'static str, Value)>,
    wheres: Vec<ExprKind>,
    returning: Vec<SelectItem>,
    _marker: PhantomData<fn() -> (D, T, Sel)>,
}

impl<D: Dialect, T: Table, Sel> UpdateReturning<D, T, Sel> {
    pub fn to_sql(&self) -> (String, Vec<Value>) {
        let mut sink = render_set_clause::<D, T>(&self.sets, &self.wheres);
        sink.text(" RETURNING ");
        render_select_list::<D>(&self.returning, &mut sink);
        sink.finish()
    }
}

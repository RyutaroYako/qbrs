//! `UPDATE .. SET .. WHERE ..`.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsReturning};
use crate::expr::{Bool, Expr, ExprKind, Value};
use crate::render::{SelectItem, render_expr, render_ident, render_select_list};
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
) -> (String, Vec<Value>) {
    // An `UPDATE` with nothing set has no SQL form, and `*Update`'s derived
    // `Default` is exactly that shape — the state a PATCH handler holds when
    // the request changed nothing.
    assert!(
        !sets.is_empty(),
        "an UPDATE must set at least one column; every field of this `*Update` is untouched"
    );

    let mut sql = String::from("UPDATE ");
    render_ident::<D>(&mut sql, T::NAME);
    sql.push_str(" SET ");
    let mut params = Vec::new();
    for (i, (col, val)) in sets.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }
        render_ident::<D>(&mut sql, col);
        sql.push_str(" = ");
        params.push(val.clone());
        sql.push_str(&D::placeholder(params.len()));
    }

    if !wheres.is_empty() {
        sql.push_str(" WHERE ");
        for (i, w) in wheres.iter().enumerate() {
            if i > 0 {
                sql.push_str(" AND ");
            }
            render_expr::<D>(w, &mut sql, &mut params);
        }
    }

    (sql, params)
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
        render_set_clause::<D, T>(&self.sets, &self.wheres)
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
        let (mut sql, mut params) = render_set_clause::<D, T>(&self.sets, &self.wheres);
        sql.push_str(" RETURNING ");
        render_select_list::<D>(&self.returning, &mut sql, &mut params);
        (sql, params)
    }
}

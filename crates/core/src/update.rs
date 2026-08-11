//! `UPDATE .. SET .. WHERE ..`.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsReturning};
use crate::expr::{Bool, Expr, ExprKind, Value};
use crate::render::{render_expr, render_ident};
use crate::scope::{Cons, Nil, NotNull, Superset, Table, TableSlot};
use crate::select::Selection;

/// Implemented by the `#[derive(Table)]`-generated `*Update` struct: every
/// field is optional in the *outer* sense (untouched vs. touched), and for
/// nullable columns doubly-optional (untouched vs. explicit NULL vs.
/// explicit value) — see the design plan's Update-struct rule. `sets()`
/// returns only the touched `(column, value)` pairs.
pub trait UpdateRow {
    type Table: Table;
    fn sets(self) -> Vec<(&'static str, Value)>;
}

pub struct UpdateSeed<D, T> {
    _marker: PhantomData<fn() -> (D, T)>,
}

pub fn update<D, T: Table>(_table: T) -> UpdateSeed<D, T> {
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
    /// See `insert::Insert::returning`'s doc comment for why this returns a
    /// distinct type rather than `Self` with a field toggled.
    pub fn returning<Sel, Idx>(self, sel: Sel) -> UpdateReturning<D, T, Sel>
    where
        Sel: Selection<Cons<TableSlot<T, NotNull>, Nil>, Idx>,
    {
        UpdateReturning {
            sets: self.sets,
            wheres: self.wheres,
            returning_exprs: sel.exprs(),
            _marker: PhantomData,
        }
    }
}

pub struct UpdateReturning<D, T: Table, Sel> {
    sets: Vec<(&'static str, Value)>,
    wheres: Vec<ExprKind>,
    returning_exprs: Vec<ExprKind>,
    _marker: PhantomData<fn() -> (D, T, Sel)>,
}

impl<D: Dialect, T: Table, Sel> UpdateReturning<D, T, Sel> {
    pub fn to_sql(&self) -> (String, Vec<Value>) {
        let (mut sql, mut params) = render_set_clause::<D, T>(&self.sets, &self.wheres);
        sql.push_str(" RETURNING ");
        for (i, e) in self.returning_exprs.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            render_expr::<D>(e, &mut sql, &mut params);
        }
        (sql, params)
    }
}

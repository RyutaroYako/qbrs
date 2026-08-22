//! `DELETE FROM .. WHERE ..`.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsReturning};
use crate::expr::{Bool, Expr, ExprKind, Value};
use crate::render::{SelectItem, render_expr, render_ident, render_select_list};
use crate::scope::{Cons, Nil, NotNull, Superset, Table, TableSlot};
use crate::select::Selection;

pub fn delete<D, T: Table>(_table: T) -> Delete<D, T> {
    Delete {
        wheres: Vec::new(),
        _marker: PhantomData,
    }
}

fn render_delete<D: Dialect, T: Table>(wheres: &[ExprKind]) -> (String, Vec<Value>) {
    let mut sql = String::from("DELETE FROM ");
    render_ident::<D>(&mut sql, T::NAME);
    let mut params = Vec::new();

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

pub struct Delete<D, T: Table> {
    wheres: Vec<ExprKind>,
    _marker: PhantomData<fn() -> (D, T)>,
}

impl<D, T: Table> Delete<D, T> {
    pub fn filter<Req, Idxs>(mut self, cond: Expr<Req, Bool>) -> Self
    where
        Cons<TableSlot<T, NotNull>, Nil>: Superset<Req, Idxs>,
    {
        self.wheres.push(cond.kind);
        self
    }
}

impl<D: Dialect, T: Table> Delete<D, T> {
    pub fn to_sql(&self) -> (String, Vec<Value>) {
        render_delete::<D, T>(&self.wheres)
    }
}

impl<D: SupportsReturning, T: Table> Delete<D, T> {
    /// See `insert::Insert::returning`'s doc comment for why this returns a
    /// distinct type rather than `Self` with a field toggled.
    pub fn returning<Sel, Idx>(self, sel: Sel) -> DeleteReturning<D, T, Sel>
    where
        Sel: Selection<Cons<TableSlot<T, NotNull>, Nil>, Idx>,
    {
        DeleteReturning {
            wheres: self.wheres,
            returning: sel.items(),
            _marker: PhantomData,
        }
    }
}

pub struct DeleteReturning<D, T: Table, Sel> {
    wheres: Vec<ExprKind>,
    returning: Vec<SelectItem>,
    _marker: PhantomData<fn() -> (D, T, Sel)>,
}

impl<D: Dialect, T: Table, Sel> DeleteReturning<D, T, Sel> {
    pub fn to_sql(&self) -> (String, Vec<Value>) {
        let (mut sql, mut params) = render_delete::<D, T>(&self.wheres);
        sql.push_str(" RETURNING ");
        render_select_list::<D>(&self.returning, &mut sql, &mut params);
        (sql, params)
    }
}

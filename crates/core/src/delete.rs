//! `DELETE FROM .. WHERE ..`.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsReturning};
use crate::expr::{Bool, Expr, ExprKind, Value};
use crate::render::{
    QuerySink, SelectItem, Sink, render_and_list, render_ident, render_select_list,
};
use crate::scope::{BaseTable, Cons, Nil, NotNull, Superset, Table, TableSlot};
use crate::select::{Predicate, Selection};

pub fn delete<D, T: BaseTable>(_table: T) -> Delete<D, T> {
    Delete {
        wheres: Vec::new(),
        _marker: PhantomData,
    }
}

fn render_delete<D: Dialect, T: Table>(wheres: &[ExprKind]) -> QuerySink<D> {
    let mut sink = QuerySink::<D>::new();
    sink.text("DELETE FROM ");
    render_ident::<D>(&mut sink, T::NAME);

    render_and_list::<D>(&mut sink, " WHERE ", wheres);

    sink
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

    /// AND-folds a runtime-length collection of discharged conditions, the
    /// same way `Select::filter_all` does.
    pub fn filter_all(
        mut self,
        conds: impl IntoIterator<Item = Predicate<Cons<TableSlot<T, NotNull>, Nil>>>,
    ) -> Self {
        self.wheres
            .extend(conds.into_iter().map(Predicate::into_kind));
        self
    }
}

impl<D: Dialect, T: Table> Delete<D, T> {
    pub fn to_sql(&self) -> (String, Vec<Value>) {
        render_delete::<D, T>(&self.wheres).finish()
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
        let mut sink = render_delete::<D, T>(&self.wheres);
        sink.text(" RETURNING ");
        render_select_list::<D>(&self.returning, &mut sink);
        sink.finish()
    }
}

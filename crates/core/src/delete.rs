//! `DELETE FROM .. WHERE ..`.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsReturning};
use crate::expr::{Bool, Expr, ExprKind};
use crate::render::{QuerySink, Sink, render_and_list, render_ident};
use crate::scope::{BaseTable, Cons, Nil, NotNull, Superset, Table, TableSlot};
use crate::select::{Predicate, Selection};
use crate::statement::{Returning, Statement, WrittenTable};

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

impl<D: Dialect, T: Table> crate::statement::StatementSealed for Delete<D, T> {}

impl<D: Dialect, T: Table> Statement for Delete<D, T> {
    type Dialect = D;
    type Table = T;
    fn render(&self) -> QuerySink<D> {
        render_delete::<D, T>(&self.wheres)
    }
}

impl<D: SupportsReturning, T: Table> Delete<D, T> {
    /// A distinct type rather than `Self` with a flag set: the execution
    /// layer needs `Sel`'s concrete type to know what to decode a returned
    /// row into, and an optional field would erase it.
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

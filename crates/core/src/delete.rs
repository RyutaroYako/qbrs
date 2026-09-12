//! `DELETE FROM .. WHERE ..`.

use std::marker::PhantomData;

use crate::dialect::Dialect;
use crate::expr::ExprKind;
use crate::render::{Sink, render_and_list, render_ident};
use crate::scope::{BaseTable, Table};
use crate::select::{Condition, Predicate};
use crate::statement::{Statement, WrittenTable};

pub fn delete<D, T: BaseTable>(_table: T) -> Delete<D, T> {
    Delete {
        wheres: Vec::new(),
        _marker: PhantomData,
    }
}

fn render_delete<D: Dialect, T: Table>(sink: &mut dyn Sink, wheres: &[ExprKind]) {
    sink.text("DELETE FROM ");
    render_ident::<D>(sink, T::NAME);
    render_and_list::<D>(sink, " WHERE ", wheres);
}

pub struct Delete<D, T: Table> {
    wheres: Vec<ExprKind>,
    _marker: PhantomData<fn() -> (D, T)>,
}

impl<D, T: Table> Delete<D, T> {
    /// A correlated subquery over the table this statement writes — the
    /// same `EXISTS` a `SELECT` builds with `Select::correlated`, against
    /// the one-table scope a write statement has.
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
    /// same way `Select::filter_all` does.
    pub fn filter_all(
        mut self,
        conds: impl IntoIterator<Item = Predicate<D, WrittenTable<T>>>,
    ) -> Self {
        self.wheres
            .extend(conds.into_iter().map(Predicate::into_kind));
        self
    }
}

impl<D: Dialect, T: Table> crate::statement::private::Sealed for Delete<D, T> {}

impl<D: Dialect, T: Table> Statement for Delete<D, T> {
    type Dialect = D;
    type Table = T;
    fn render_into(&self, sink: &mut dyn Sink) {
        render_delete::<D, T>(sink, &self.wheres);
    }
}

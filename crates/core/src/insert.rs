//! `INSERT INTO .. VALUES ..`.
//!
//! A column with a schema default gets a `Defaultable<T>` field, so "omit"
//! and "explicit value" stay distinguishable — and `Defaultable<Option<T>>`
//! when it's also nullable, making that three distinct states. Omission
//! renders as the `DEFAULT` keyword in that row's `VALUES (..)` tuple rather
//! than changing the column list, so rows that omit different fields still
//! share one statement.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsOnConflict, SupportsReturning};
use crate::expr::{Column, ColumnKey, Value};
use crate::render::{QuerySink, Sink, render_ident};
use crate::scope::{BaseTable, Table};
use crate::select::Selection;
use crate::statement::{Returning, Statement, WrittenTable};
use crate::update::{Assignments, NothingToSet, UpdateRow};

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Defaultable<T> {
    #[default]
    Default,
    Value(T),
}

impl<T> Defaultable<T> {
    pub fn value(v: T) -> Self {
        Defaultable::Value(v)
    }
}

/// A request struct's `Option<T>` field maps onto an insert's three-state
/// column the one way that makes sense — absent means "let the schema
/// decide" — so a `POST` body reaches an `*Insert` field-for-field, the way
/// a `PATCH` body already reaches an `*Update`.
impl<T> From<Option<T>> for Defaultable<T> {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(v) => Defaultable::Value(v),
            None => Defaultable::Default,
        }
    }
}

#[derive(Debug, Clone)]
pub enum InsertValue {
    Value(Value),
    /// Renders as the bare `DEFAULT` keyword in the `VALUES` list.
    Default,
}

impl<T: Into<Value>> From<Defaultable<T>> for InsertValue {
    fn from(d: Defaultable<T>) -> Self {
        match d {
            Defaultable::Default => InsertValue::Default,
            Defaultable::Value(v) => InsertValue::Value(v.into()),
        }
    }
}

/// Implemented by the `#[derive(Table)]`-generated `*Insert` struct for each
/// table, which is what pairs `COLUMNS` with a matching `into_values()`
/// order and length; a hand-written impl has to keep the two in step
/// itself.
pub trait InsertRow {
    type Table: Table;
    const COLUMNS: &'static [&'static str];
    fn into_values(self) -> Vec<InsertValue>;
}

/// An `ON CONFLICT` target: one or more columns proven by `T` to belong to
/// the table being inserted into, where a raw `&[&str]` would let a typo
/// through to the database. Implemented for a bare `Column<C>` and for
/// tuples of up to three; add arities as real schemas need them.
pub trait ConflictTarget<T: Table> {
    fn column_names(&self) -> Vec<&'static str>;
}

impl<C: ColumnKey> ConflictTarget<C::Table> for Column<C> {
    fn column_names(&self) -> Vec<&'static str> {
        vec![C::NAME]
    }
}

impl<T: Table, A: ConflictTarget<T>> ConflictTarget<T> for (A,) {
    fn column_names(&self) -> Vec<&'static str> {
        self.0.column_names()
    }
}

impl<T: Table, A: ConflictTarget<T>, B: ConflictTarget<T>> ConflictTarget<T> for (A, B) {
    fn column_names(&self) -> Vec<&'static str> {
        let mut v = self.0.column_names();
        v.extend(self.1.column_names());
        v
    }
}

impl<T: Table, A: ConflictTarget<T>, B: ConflictTarget<T>, C: ConflictTarget<T>> ConflictTarget<T>
    for (A, B, C)
{
    fn column_names(&self) -> Vec<&'static str> {
        let mut v = self.0.column_names();
        v.extend(self.1.column_names());
        v.extend(self.2.column_names());
        v
    }
}

enum ConflictAction {
    DoNothing,
    /// Reuses `UpdateRow`, so `.on_conflict_do_update(..)` takes the same
    /// `*Update` value `.set(..)` does.
    ///
    /// **Known limitation**: only literal/bound values, not
    /// `EXCLUDED.column` (`SET total = users.total + EXCLUDED.total`), which
    /// needs its own typed API.
    DoUpdate(Assignments),
}

struct ConflictClause {
    target: Vec<&'static str>,
    action: ConflictAction,
}

fn render_conflict_clause<D: Dialect>(clause: &ConflictClause, sink: &mut dyn Sink) {
    sink.text(" ON CONFLICT (");
    for (i, c) in clause.target.iter().enumerate() {
        if i > 0 {
            sink.text(", ");
        }
        render_ident::<D>(sink, c);
    }
    sink.ch(')');
    match &clause.action {
        ConflictAction::DoNothing => sink.text(" DO NOTHING"),
        ConflictAction::DoUpdate(sets) => {
            sink.text(" DO UPDATE SET ");
            sets.render_into::<D>(sink);
        }
    }
}

pub struct InsertSeed<D, T> {
    _marker: PhantomData<fn() -> (D, T)>,
}

pub fn insert<D, T: BaseTable>(_table: T) -> InsertSeed<D, T> {
    InsertSeed {
        _marker: PhantomData,
    }
}

impl<D, T: Table> InsertSeed<D, T> {
    pub fn values<R: InsertRow<Table = T>>(self, row: R) -> Insert<D, R> {
        Insert {
            rows: vec![row.into_values()],
            on_conflict: None,
            _marker: PhantomData,
        }
    }

    /// Every row of a collection at once — the shape a bulk import has,
    /// where the rows are already in a `Vec` and the first one isn't
    /// special. `INSERT` with no rows has no SQL form, so an empty
    /// collection is refused here rather than rendered.
    pub fn values_all<R: InsertRow<Table = T>>(
        self,
        rows: impl IntoIterator<Item = R>,
    ) -> Result<Insert<D, R>, NothingToInsert> {
        let rows: Vec<_> = rows.into_iter().map(R::into_values).collect();
        if rows.is_empty() {
            return Err(NothingToInsert);
        }
        Ok(Insert {
            rows,
            on_conflict: None,
            _marker: PhantomData,
        })
    }
}

/// An `INSERT` was given no rows at all. Returned rather than panicked for
/// the reason `update::NothingToSet` gives: an empty collection is ordinary
/// request-shaped data, and the caller decides whether it is a no-op or an
/// error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NothingToInsert;

impl std::fmt::Display for NothingToInsert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an INSERT must have at least one row, but no rows were given")
    }
}
impl std::error::Error for NothingToInsert {}

fn render_values_clause<D: Dialect, R: InsertRow>(
    rows: &[Vec<InsertValue>],
    on_conflict: &Option<ConflictClause>,
) -> QuerySink<D> {
    let mut sink = QuerySink::<D>::new();
    sink.text("INSERT INTO ");
    render_ident::<D>(&mut sink, <R::Table as Table>::NAME);
    sink.text(" (");
    for (i, c) in R::COLUMNS.iter().enumerate() {
        if i > 0 {
            sink.text(", ");
        }
        render_ident::<D>(&mut sink, c);
    }
    sink.text(") VALUES ");

    for (row_i, row) in rows.iter().enumerate() {
        if row_i > 0 {
            sink.text(", ");
        }
        sink.ch('(');
        for (i, v) in row.iter().enumerate() {
            if i > 0 {
                sink.text(", ");
            }
            match v {
                InsertValue::Default => sink.text("DEFAULT"),
                InsertValue::Value(v) => sink.bind(v),
            }
        }
        sink.ch(')');
    }

    if let Some(clause) = on_conflict {
        render_conflict_clause::<D>(clause, &mut sink);
    }

    sink
}

pub struct Insert<D, R: InsertRow> {
    rows: Vec<Vec<InsertValue>>,
    on_conflict: Option<ConflictClause>,
    _marker: PhantomData<fn() -> (D, R)>,
}

impl<D, R: InsertRow> Insert<D, R> {
    /// Bulk insert: add another row to the same statement.
    pub fn values(mut self, row: R) -> Self {
        self.rows.push(row.into_values());
        self
    }
}

impl<D: SupportsOnConflict, R: InsertRow> Insert<D, R> {
    /// `ON CONFLICT (..) DO NOTHING`.
    pub fn on_conflict_do_nothing(mut self, target: impl ConflictTarget<R::Table>) -> Self {
        self.on_conflict = Some(ConflictClause {
            target: target.column_names(),
            action: ConflictAction::DoNothing,
        });
        self
    }

    /// `ON CONFLICT (..) DO UPDATE SET ..`, reusing the same `*Update`
    /// struct `update().set(..)` takes.
    pub fn on_conflict_do_update<U: UpdateRow<Table = R::Table>>(
        mut self,
        target: impl ConflictTarget<R::Table>,
        set: U,
    ) -> Result<Self, NothingToSet> {
        self.on_conflict = Some(ConflictClause {
            target: target.column_names(),
            action: ConflictAction::DoUpdate(Assignments::new(set)?),
        });
        Ok(self)
    }
}

impl<D: Dialect, R: InsertRow> crate::statement::StatementSealed for Insert<D, R> {}

impl<D: Dialect, R: InsertRow> Statement for Insert<D, R> {
    type Dialect = D;
    type Table = R::Table;
    fn render(&self) -> QuerySink<D> {
        render_values_clause::<D, R>(&self.rows, &self.on_conflict)
    }
}

impl<D: SupportsReturning, R: InsertRow> Insert<D, R> {
    /// A distinct type rather than `Self` with a flag set, for the reason
    /// `delete::Delete::returning` gives.
    pub fn returning<Sel, Idx>(self, sel: Sel) -> Returning<Self, Sel>
    where
        Sel: Selection<WrittenTable<R::Table>, Idx>,
    {
        Returning {
            returning: sel.items(),
            statement: self,
            _marker: PhantomData,
        }
    }
}

//! `INSERT INTO .. VALUES ..`.
//!
//! A column with a schema default gets a `Defaultable<T>` field, so "omit"
//! and "explicit value" stay distinguishable — and `Defaultable<Option<T>>`
//! when it's also nullable, making that three distinct states. Omission
//! renders as the `DEFAULT` keyword in that row's `VALUES (..)` tuple rather
//! than changing the column list, so rows that omit different fields still
//! share one statement.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsOnConflict};
use crate::expr::{Column, ColumnKey, Value};
use crate::render::{QuerySink, Sink, render_ident};
use crate::scope::{BaseTable, Table};
use crate::statement::Statement;
use crate::update::Assignments;

/// What a column's setter accepts, keyed by what that column's field
/// holds: the column's own Rust type — `&str` for text — plus the `Option`
/// a request struct already carries, wherever leaving the column out means
/// something. What `None` means is the position's own: on an insert it is
/// NULL for a nullable column and the schema's default for a defaulted one
/// (`.<column>_null()` says the other, where a column is both); on an
/// update it is *untouched*, since an `UPDATE` that says nothing about a
/// column leaves it alone.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a value this column accepts",
    label = "expected the column's own Rust type, or an `Option` of it"
)]
pub trait IntoColumnValue<V> {
    fn into_column_value(self) -> V;
}

mod insertable {
    /// Sealed like the other markers a derive emits. Unlike `Filled`, whose
    /// doc explains why forging it buys nothing, forging this one turns a
    /// bulk insert into a single `DEFAULT VALUES` — rows lost quietly — so
    /// it gets the door even though a determined caller can still write it.
    pub trait Sealed {}
}

#[doc(hidden)]
pub use insertable::Sealed as InsertableSealed;

/// A table with at least one column a statement may insert into. Emitted by
/// `#[derive(Table)]` unless every column is generated, which leaves an
/// `INSERT` with nothing to name: SQL spells that `DEFAULT VALUES`, and
/// spells it for exactly one row.
#[diagnostic::on_unimplemented(
    message = "every column of `{Self}`'s table is generated, so only one row at a time can be inserted",
    label = "`DEFAULT VALUES` is what SQL calls a row with nothing in it, and it names no columns to repeat",
    note = "insert them one statement at a time"
)]
pub trait Insertable: InsertableSealed {}

/// Proof that a builder's slot for column `C` holds that column's value.
/// Deliberately unsealed, unlike `scope::Find`: forging it buys nothing,
/// because `*Insert`'s fields are public and a complete row with a value of
/// the caller's choosing is directly constructible. What the type-state
/// builder prevents is *forgetting* a column, not choosing its value — and
/// a seal here can't hold anyway, since the derive must implement this in
/// the schema's own crate, where any nameable proof is nameable twice.
/// `Missing<C>` doesn't implement it, which is what `build()` is bounded
/// by — on the method rather than by the slot's type, so an incomplete row
/// is a sentence naming the column rather than a missing `build`.
#[diagnostic::on_unimplemented(
    message = "column `{C}` hasn't been given a value yet",
    label = "every column that is neither nullable nor defaulted needs one before `.build()`"
)]
pub trait Filled<C> {
    #[doc(hidden)]
    type Value;
    #[doc(hidden)]
    fn filled(self) -> Self::Value;
}

/// A column an `*Insert` builder hasn't been given a value for yet. Named
/// after the column so the builder's type says which one is missing, rather
/// than leaving a bare `()` to be counted by position.
pub struct Missing<C>(std::marker::PhantomData<fn() -> C>);

impl<C> Missing<C> {
    #[doc(hidden)]
    pub const fn new() -> Self {
        Missing(std::marker::PhantomData)
    }
}

impl<C> Default for Missing<C> {
    fn default() -> Self {
        Missing::new()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Defaultable<T> {
    #[default]
    Default,
    Value(T),
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
/// table. One list of `(column, value)` pairs rather than a name list beside
/// a value list, for the reason `select::AllColumns` carries one list: the
/// seal is `#[doc(hidden)] pub` — the derive has to write it in the schema's
/// own crate — so two lists that have to line up position for position
/// could be made not to, and `INSERT INTO t (a, b, c) VALUES ($1)` is
/// malformed whatever the table looks like. `update::UpdateRow::sets` has
/// always had this shape.
pub trait InsertRow: private::Sealed {
    type Table: Table;

    /// This row's columns *and* values as one chain: the keys spell the
    /// statement's header, the cells carry what goes under them. One type,
    /// because two lists reconciled at render time is the shape that made
    /// every earlier version of this trait able to produce SQL malformed
    /// for any table, or to drop a value silently — a `COLUMNS` const
    /// beside positional values, then pairs matched against the first row's
    /// names, then pairs matched against a declared header. A chain can
    /// carry neither a surplus cell nor a missing one.
    type Values: InsertValues;

    fn into_values(self) -> Self::Values;
}

mod insert_values {
    /// Sealed to the two shapes a chain has, so `Values` is always a real
    /// one — the reason `row::ColumnNames` is sealed.
    pub trait Sealed {}
}

/// One row's cells, in the order its chain declares them.
fn collect_values<R: InsertRow>(row: R) -> Vec<InsertValue> {
    let mut out = Vec::new();
    row.into_values().push_values(&mut out);
    out
}

/// A chain of `(column, value)` cells: `RowCons<C, InsertValue, Tail>` down
/// to `RowNil`. Walked once for the header and once for each row's values,
/// so the two cannot disagree.
pub trait InsertValues: insert_values::Sealed {
    #[doc(hidden)]
    fn push_names(out: &mut Vec<&'static str>);
    #[doc(hidden)]
    fn push_values(self, out: &mut Vec<InsertValue>);
}

impl insert_values::Sealed for crate::row::RowNil {}

impl InsertValues for crate::row::RowNil {
    fn push_names(_out: &mut Vec<&'static str>) {}
    fn push_values(self, _out: &mut Vec<InsertValue>) {}
}

impl<C: crate::row::Named, Tail: InsertValues> insert_values::Sealed
    for crate::row::RowCons<C, InsertValue, Tail>
{
}

impl<C: crate::row::Named, Tail: InsertValues> InsertValues
    for crate::row::RowCons<C, InsertValue, Tail>
{
    fn push_names(out: &mut Vec<&'static str>) {
        out.push(<C as crate::row::Named>::NAME);
        Tail::push_names(out);
    }

    fn push_values(self, out: &mut Vec<InsertValue>) {
        let (value, tail) = self.into_cell();
        out.push(value);
        tail.push_values(out);
    }
}

/// An `ON CONFLICT` target: one or more columns proven by `T` to belong to
/// the table being inserted into, where a raw `&[&str]` would let a typo
/// through to the database. Implemented for a bare `Column<C>` and for
/// tuples of up to three; add arities as real schemas need them.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't an `ON CONFLICT` target for `{T}`",
    label = "a column of that table, or a tuple of up to three of them"
)]
pub trait ConflictTarget<T: Table>: conflict_target::Sealed {
    #[doc(hidden)]
    fn column_names(&self) -> Vec<&'static str>;
}

mod conflict_target {
    /// Sealed for the reason `InsertRow` is: a hand-written impl could name
    /// a column that isn't there, and the point of taking `Column<C>`s is
    /// that it can't.
    pub trait Sealed {}
    impl<C: crate::expr::ColumnKey> Sealed for crate::expr::Column<C> {}
    // Elements constrained, or the seal admits a tuple of anything — and a
    // hand-written `ConflictTarget` for it could name a column that isn't
    // there, which is what this seal is for.
    impl<A: crate::expr::ColumnKey> Sealed for (crate::expr::Column<A>,) {}
    impl<A: crate::expr::ColumnKey, B: crate::expr::ColumnKey> Sealed
        for (crate::expr::Column<A>, crate::expr::Column<B>)
    {
    }
    impl<A: crate::expr::ColumnKey, B: crate::expr::ColumnKey, C: crate::expr::ColumnKey> Sealed
        for (
            crate::expr::Column<A>,
            crate::expr::Column<B>,
            crate::expr::Column<C>,
        )
    {
    }
}

impl<C: ColumnKey> ConflictTarget<C::Table> for Column<C> {
    fn column_names(&self) -> Vec<&'static str> {
        vec![C::NAME]
    }
}

macro_rules! conflict_target_tuple {
    ($($name:ident),+) => {
        #[allow(non_snake_case)]
        impl<T: Table, $($name: ColumnKey<Table = T>,)+> ConflictTarget<T> for ($(Column<$name>,)+) {
            fn column_names(&self) -> Vec<&'static str> {
                vec![$(<$name as crate::row::Named>::NAME),+]
            }
        }
    };
}
// Columns, not nested targets: a conflict target is a list of columns, and
// letting it nest is what made the documented limit of three not one.
conflict_target_tuple!(A);
conflict_target_tuple!(A, B);
conflict_target_tuple!(A, B, C);

enum ConflictAction<T> {
    DoNothing,
    /// The same `SET` list `UPDATE` takes: an `*Update` value, or
    /// `Assignments` of expressions.
    ///
    /// **Known limitation**: no `EXCLUDED.column` (`SET total = total +
    /// EXCLUDED.total`) — the row being inserted isn't a table the scope
    /// knows, so referring to it needs its own typed API. Everything else a
    /// `SET` list can say, including expressions over the target's own
    /// columns, works here.
    DoUpdate(Assignments<T>),
}

struct ConflictClause<T> {
    target: Vec<&'static str>,
    action: ConflictAction<T>,
}

fn render_conflict_clause<D: Dialect, T>(clause: &ConflictClause<T>, sink: &mut dyn Sink) {
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

mod private {
    /// The row's `(column, value)` pairs are what a statement writes;
    /// `#[derive(Table)]` is what guarantees that, so it is the only thing
    /// that can produce an `InsertRow`.
    pub trait Sealed {}
}

#[doc(hidden)]
pub use private::Sealed as InsertRowSealed;

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
            rows: vec![collect_values(row)],
            on_conflict: None,
            _marker: PhantomData,
        }
    }

    /// Every row of a collection at once — the shape a bulk import has,
    /// where the rows are already in a `Vec` and the first one isn't
    /// special. `INSERT` with no rows has no SQL form, so an empty
    /// collection is refused here rather than rendered.
    pub fn values_all<R: InsertRow<Table = T> + Insertable>(
        self,
        rows: impl IntoIterator<Item = R>,
    ) -> Result<Insert<D, R>, NothingToInsert> {
        let rows: Vec<_> = rows.into_iter().map(collect_values).collect();
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
    on_conflict: &Option<ConflictClause<R::Table>>,
) -> QuerySink<D> {
    let mut sink = QuerySink::<D>::new();
    sink.text("INSERT INTO ");
    render_ident::<D>(&mut sink, <R::Table as Table>::NAME);

    // Header and values come from one chain, so a row has exactly one cell
    // per column named — no reconciliation, nothing to drop.
    let mut header = Vec::new();
    <R::Values as InsertValues>::push_names(&mut header);

    // A table whose every column is generated leaves nothing to name, and
    // an empty column list is a syntax error in two of the three dialects.
    // One such row is all SQL can express, which is why `values`/`values_all`
    // take `Insertable`.
    if header.is_empty() {
        sink.text(D::INSERT_NO_COLUMNS);
        if let Some(clause) = on_conflict {
            render_conflict_clause::<D, _>(clause, &mut sink);
        }
        return sink;
    }

    sink.text(" (");
    for (i, name) in header.iter().enumerate() {
        if i > 0 {
            sink.text(", ");
        }
        render_ident::<D>(&mut sink, name);
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
        render_conflict_clause::<D, _>(clause, &mut sink);
    }

    sink
}

pub struct Insert<D, R: InsertRow> {
    rows: Vec<Vec<InsertValue>>,
    on_conflict: Option<ConflictClause<R::Table>>,
    _marker: PhantomData<fn() -> (D, R)>,
}

impl<D, R: InsertRow + Insertable> Insert<D, R> {
    /// Bulk insert: add another row to the same statement.
    pub fn values(mut self, row: R) -> Self {
        self.rows.push(collect_values(row));
        self
    }

    /// The same for a collection. Infallible, unlike the seed's: this
    /// statement already has a row, so an empty collection adds nothing
    /// rather than describing an `INSERT` with nothing in it.
    pub fn values_all(mut self, rows: impl IntoIterator<Item = R>) -> Self {
        self.rows.extend(rows.into_iter().map(collect_values));
        self
    }
}

impl<D: Dialect, R: InsertRow> Insert<D, R> {
    /// `ON CONFLICT (..) DO NOTHING`.
    pub fn on_conflict_do_nothing(mut self, target: impl ConflictTarget<R::Table>) -> Self
    where
        D: SupportsOnConflict,
    {
        self.on_conflict = Some(ConflictClause {
            target: target.column_names(),
            action: ConflictAction::DoNothing,
        });
        self
    }

    /// `ON CONFLICT (..) DO UPDATE SET ..`, taking the same `Assignments`
    /// an `UPDATE` sets.
    pub fn on_conflict_do_update(
        mut self,
        target: impl ConflictTarget<R::Table>,
        set: Assignments<R::Table>,
    ) -> Self
    where
        D: SupportsOnConflict,
    {
        self.on_conflict = Some(ConflictClause {
            target: target.column_names(),
            action: ConflictAction::DoUpdate(set),
        });
        self
    }
}

impl<D: Dialect, R: InsertRow> crate::statement::private::Sealed for Insert<D, R> {}

impl<D: Dialect, R: InsertRow> Statement for Insert<D, R> {
    type Dialect = D;
    type Table = R::Table;
    fn render(&self) -> QuerySink<D> {
        render_values_clause::<D, R>(&self.rows, &self.on_conflict)
    }
}

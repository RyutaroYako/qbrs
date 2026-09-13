//! `INSERT INTO ..`, from values or from a query.
//!
//! A column with a schema default gets a `Defaultable<T>` field, so "omit"
//! and "explicit value" stay distinguishable. One that is nullable as well
//! gets `Defaultable<Option<T>>`, making that three distinct states. Omission renders as the `DEFAULT` keyword in that row's
//! `VALUES (..)` tuple rather than changing the column list, so rows that
//! omit different fields still share one statement.
//!
//! An `ON CONFLICT` target names columns, and the database infers an index
//! from them: one over exactly those columns whose own predicate the
//! target's implies. A *partial* unique index therefore needs its predicate
//! repeated, which is what `partial_index(..)` is for.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsOnConflict};
use crate::expr::{AssignsTo, Column, ColumnKey, Expr, ExprKind, IntoExpr, Value, Writable};
use crate::render::{Sink, render_ident};
use crate::scope::{BaseTable, Cons, Nil, NotNull, Superset, Table, TableSlot};
use crate::select::{Condition, Predicate};
use crate::statement::{Statement, WrittenTable};
use crate::update::Assignments;

/// What a column's setter accepts, keyed by what that column's field
/// holds: the column's own Rust type (`&str` for text), plus the `Option`
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
    /// bulk insert into a single `DEFAULT VALUES`, losing rows quietly, so
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
/// builder prevents is *forgetting* a column, not choosing its value. A
/// seal here can't hold anyway, since the derive must implement this in
/// the schema's own crate, where any nameable proof is nameable twice.
/// `Missing<C>` doesn't implement it, which is what `build()` is bounded
/// by. That bound sits on the method rather than on the impl block, so an
/// incomplete row is a sentence naming the column rather than a missing
/// `build`.
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
/// seal is `#[doc(hidden)] pub`, since the derive has to write it in the
/// schema's own crate. Two lists that have to line up position for position
/// could therefore be made not to, and `INSERT INTO t (a, b, c) VALUES ($1)`
/// is malformed whatever the table looks like. `update::UpdateRow::sets` has
/// always had this shape.
pub trait InsertRow: private::Sealed {
    type Table: Table;

    /// This row's columns *and* values as one chain: the keys spell the
    /// statement's header, the cells carry what goes under them. One type,
    /// because two lists reconciled at render time is the shape that made
    /// every earlier version of this trait able to produce SQL malformed
    /// for any table, or to drop a value silently: a `COLUMNS` const
    /// beside positional values, then pairs matched against the first row's
    /// names, then pairs matched against a declared header. A chain can
    /// carry neither a surplus cell nor a missing one.
    type Values: InsertValues;

    fn into_values(self) -> Self::Values;
}

mod insert_values {
    /// Sealed to the two shapes a chain has, so `Values` is always a real
    /// one. `row::ColumnNames` is sealed for the same reason.
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

/// The columns an `ON CONFLICT` target infers an index from: one or more,
/// proven by `T` to belong to the table being inserted into, where a raw
/// `&[&str]` would let a typo through to the database. Implemented for a
/// bare `Column<C>` and for tuples of up to three; add arities as real
/// schemas need them.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a column list for `{T}`",
    label = "a column of that table, or a tuple of up to three of them"
)]
pub trait ConflictColumns<T: Table>: conflict_target::ColumnsSealed<T> {
    #[doc(hidden)]
    fn column_names(&self) -> Vec<&'static str>;
}

/// An `ON CONFLICT` target: the columns, and for a partial unique index the
/// predicate that picks it. Implemented for everything `ConflictColumns`
/// is, plus the [`partial_index`] those columns pass through.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't an `ON CONFLICT` target for `{T}`",
    label = "a column of that table, a tuple of up to three of them, or `partial_index(..)` of either"
)]
pub trait ConflictTarget<T: Table>: conflict_target::Sealed<T> {
    #[doc(hidden)]
    fn into_target(self) -> Target;
}

/// The rendered half of a conflict target: the columns a dialect infers an
/// index from, and the `index_predicate` that picks a *partial* one.
#[doc(hidden)]
pub struct Target {
    columns: Vec<&'static str>,
    index_predicate: Option<ExprKind>,
}

mod conflict_target {
    /// Sealed for the reason `InsertRow` is: a hand-written impl could name
    /// a column that isn't there, and the point of taking `Column<C>`s is
    /// that it can't. Both seals carry the trait's own table parameter. A
    /// seal on `Self` alone leaves that table a free slot the caller fills
    /// with their own type, which is all the orphan rule asks for, and
    /// `ON CONFLICT ("nickname")` against a table without one is exactly
    /// the typo the seal is here to stop.
    pub trait ColumnsSealed<T> {}
    pub trait Sealed<T> {}
}

impl<C: ColumnKey> conflict_target::ColumnsSealed<C::Table> for Column<C> {}
impl<C: ColumnKey> conflict_target::Sealed<C::Table> for Column<C> {}

impl<C: ColumnKey> ConflictColumns<C::Table> for Column<C> {
    fn column_names(&self) -> Vec<&'static str> {
        vec![C::NAME]
    }
}

impl<C: ColumnKey> ConflictTarget<C::Table> for Column<C> {
    fn into_target(self) -> Target {
        Target {
            columns: self.column_names(),
            index_predicate: None,
        }
    }
}

macro_rules! conflict_target_tuple {
    ($($name:ident),+) => {
        // Elements constrained, or the seal admits a tuple of anything.
        impl<T: Table, $($name: ColumnKey<Table = T>,)+> conflict_target::ColumnsSealed<T>
            for ($(Column<$name>,)+) {}
        impl<T: Table, $($name: ColumnKey<Table = T>,)+> conflict_target::Sealed<T>
            for ($(Column<$name>,)+) {}

        #[allow(non_snake_case)]
        impl<T: Table, $($name: ColumnKey<Table = T>,)+> ConflictColumns<T> for ($(Column<$name>,)+) {
            fn column_names(&self) -> Vec<&'static str> {
                vec![$(<$name as crate::row::Named>::NAME),+]
            }
        }

        impl<T: Table, $($name: ColumnKey<Table = T>,)+> ConflictTarget<T> for ($(Column<$name>,)+) {
            fn into_target(self) -> Target {
                Target {
                    columns: self.column_names(),
                    index_predicate: None,
                }
            }
        }
    };
}
// Columns, not nested targets: a conflict target is a list of columns, and
// letting it nest is what made the documented limit of three not one.
conflict_target_tuple!(A);
conflict_target_tuple!(A, B);
conflict_target_tuple!(A, B, C);

/// `ON CONFLICT (a, b) WHERE deleted_at IS NULL`: the conflict target of a
/// **partial** unique index.
///
/// A target of bare columns is matched against an index over exactly those
/// columns whose own predicate the target's implies. A target with no
/// predicate implies only an index with none, so a partial index is
/// unreachable without one. Implication, not equality: a predicate saying
/// more than the index's still picks it. This is Postgres's
/// `index_predicate`, and SQLite spells it the same way. It is only ever
/// that: it does not filter which rows the conflict applies to, and where
/// the columns also carry an unfiltered unique index that one still wins.
///
/// A predicate that implies no index at all is refused by the database
/// rather than silently matching a different one.
///
/// ```ignore
/// insert(members::Table)
///     .values(row)
///     .on_conflict_do_update(
///         partial_index((members::team, members::handle), members::left_at.is_null()),
///         assignments,
///     )
/// ```
pub fn partial_index<T, Cols, E, Req, Idxs>(columns: Cols, index_predicate: E) -> PartialIndex<T>
where
    T: Table,
    Cols: ConflictColumns<T>,
    E: crate::expr::IntoExpr<Req = Req>,
    E::Sql: crate::expr::BoolLike,
    WrittenTable<T>: crate::scope::Superset<Req, Idxs>,
{
    PartialIndex {
        target: Target {
            columns: columns.column_names(),
            index_predicate: Some(index_predicate.into_expr().kind),
        },
        _marker: PhantomData,
    }
}

/// A conflict target narrowed to a partial unique index, from
/// [`partial_index`]. It takes the columns rather than another target, so
/// the predicate it carries is the only one there is.
pub struct PartialIndex<T> {
    target: Target,
    _marker: PhantomData<fn() -> T>,
}

impl<T> conflict_target::Sealed<T> for PartialIndex<T> {}

impl<T: Table> ConflictTarget<T> for PartialIndex<T> {
    fn into_target(self) -> Target {
        self.target
    }
}

/// How the pseudo-table is spelled, in the one place a statement renders it
/// and the one place the scope names it.
pub(crate) const EXCLUDED: &str = "excluded";

/// The row an `INSERT` proposed, as `ON CONFLICT DO UPDATE` sees it: a
/// pseudo-table holding the target's own columns. Deliberately not a
/// [`BaseTable`], so it is not a table a query can select from.
/// [`excluded`] is the only way to name a column of it.
///
/// **Known limitation**: a schema table actually called `excluded` shadows
/// nothing and is shadowed by this one, so an assignment naming that table
/// in a `DO UPDATE` reads the proposed row. This is the general case of the
/// one [`scope`](crate::scope) documents: two tables sharing a name are
/// one table to the renderer.
pub struct Excluded<T>(PhantomData<fn() -> T>);

impl<T: Table> Table for Excluded<T> {
    const NAME: &'static str = EXCLUDED;
}

/// What a `DO UPDATE` assignment may name: the conflicting row and the
/// proposed one. An `UPDATE`'s own `SET` list is discharged against
/// [`WrittenTable`] alone, which is what keeps [`excluded`] out of a
/// statement that has no proposed row.
pub type ConflictScope<T> = Cons<TableSlot<Excluded<T>, NotNull>, WrittenTable<T>>;

/// `excluded.column`: the value the column would have taken had the row
/// inserted, which is what `SET total = total + EXCLUDED.total` and the
/// plain `SET v = EXCLUDED.v` of every upsert are written with. Reads as an
/// ordinary expression over the target's columns, so it composes with them
/// through everything that takes one; what confines it is the scope it is
/// discharged against, and [`ConflictUpdate`] is the only list with it.
pub fn excluded<C: ColumnKey>(_column: Column<C>) -> Expr<Cons<Excluded<C::Table>, Nil>, C::Sql> {
    Expr::from_kind(ExprKind::Excluded {
        name: <C as crate::row::Named>::NAME,
    })
}

/// The `DO UPDATE` half of an upsert: everything an `UPDATE` assigns, plus
/// [`excluded`], plus the `WHERE` that decides whether the update fires at
/// all. An [`Assignments`] converts into one, so a request-shaped patch
/// reaches an upsert as it always did; the reverse doesn't exist, which is
/// what keeps a list naming the proposed row out of a statement that has
/// none.
///
/// It carries `D` for the reason [`Predicate`] does: its `WHERE` is a
/// condition like any other, and a condition pinned to one dialect must not
/// reach a statement of another. The statement it is passed to says which
/// dialect that is, so a value built inline needs no annotation. One bound
/// to a `let` and never used does.
pub struct ConflictUpdate<D, T> {
    sets: Vec<(&'static str, ExprKind)>,
    wheres: Vec<ExprKind>,
    _marker: PhantomData<fn() -> (D, T)>,
}

impl<D, T> From<Assignments<T>> for ConflictUpdate<D, T> {
    fn from(sets: Assignments<T>) -> Self {
        ConflictUpdate {
            sets: sets.into_sets(),
            wheres: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<D, T: Table> ConflictUpdate<D, T> {
    /// `column = <expression>`, over the conflicting row and the proposed
    /// one. This is [`Assignments::set_to`] with the wider scope.
    pub fn set_to<C, V, Idxs>(_column: Column<C>, value: V) -> Self
    where
        C: ColumnKey<Table = T> + Writable,
        V: IntoExpr,
        V::Sql: AssignsTo<C::Sql>,
        ConflictScope<T>: Superset<V::Req, Idxs>,
    {
        ConflictUpdate {
            sets: vec![(<C as crate::row::Named>::NAME, value.into_expr().kind)],
            wheres: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// One more of them.
    pub fn and_set_to<C, V, Idxs>(mut self, _column: Column<C>, value: V) -> Self
    where
        C: ColumnKey<Table = T> + Writable,
        V: IntoExpr,
        V::Sql: AssignsTo<C::Sql>,
        ConflictScope<T>: Superset<V::Req, Idxs>,
    {
        crate::update::push_set(
            &mut self.sets,
            <C as crate::row::Named>::NAME,
            value.into_expr().kind,
        );
        self
    }

    /// `DO UPDATE SET .. WHERE <condition>`: which conflicting rows the
    /// update actually touches, over the conflicting row and the proposed
    /// one. A row the condition rejects is left as it is *and is not
    /// counted*, so a one-row upsert's `rows_affected()` answers "did this
    /// write anything" rather than always being 1. It counts the rows a
    /// statement wrote, not the branch each took: a multi-row upsert's
    /// count still folds inserts together with updates, and only zero says
    /// nothing happened.
    ///
    /// A different clause from the one [`partial_index`] carries: that one
    /// only picks which index the conflict is inferred against.
    pub fn filter<C: Condition<D, ConflictScope<T>, Idxs>, Idxs>(mut self, cond: C) -> Self {
        self.wheres.push(cond.into_predicate().into_kind());
        self
    }

    /// AND-folds a runtime-length collection of discharged conditions, the
    /// same way [`Update::filter_all`](crate::update::Update::filter_all)
    /// does.
    pub fn filter_all(
        mut self,
        conds: impl IntoIterator<Item = Predicate<D, ConflictScope<T>>>,
    ) -> Self {
        self.wheres
            .extend(conds.into_iter().map(Predicate::into_kind));
        self
    }

    /// A correlated subquery over the rows this action sees. It is the
    /// same `EXISTS` [`Update::correlated`](crate::update::Update::correlated)
    /// builds, against the conflicting row and the proposed one.
    pub fn correlated<S, InnerSel>(
        &self,
        source: S,
        selection: InnerSel,
    ) -> crate::select::Select<
        D,
        Cons<TableSlot<S::Table, NotNull>, ConflictScope<T>>,
        InnerSel,
        ConflictScope<T>,
    >
    where
        S: crate::select::JoinSource<D>,
    {
        crate::select::correlated_with(source, selection)
    }
}

impl<D: Dialect, T> ConflictUpdate<D, T> {
    fn render_into(&self, sink: &mut dyn Sink) {
        for (i, (col, value)) in self.sets.iter().enumerate() {
            if i > 0 {
                sink.text(", ");
            }
            render_ident::<D>(sink, col);
            sink.text(" = ");
            crate::render::render_expr::<D>(value, sink);
        }
        crate::render::render_and_list::<D>(sink, " WHERE ", &self.wheres);
    }
}

enum ConflictAction<D, T> {
    DoNothing,
    DoUpdate(ConflictUpdate<D, T>),
}

struct ConflictClause<D, T> {
    target: Target,
    action: ConflictAction<D, T>,
}

fn render_conflict_clause<D: Dialect, T>(clause: &ConflictClause<D, T>, sink: &mut dyn Sink) {
    sink.text(" ON CONFLICT (");
    for (i, c) in clause.target.columns.iter().enumerate() {
        if i > 0 {
            sink.text(", ");
        }
        render_ident::<D>(sink, c);
    }
    sink.ch(')');
    if let Some(predicate) = &clause.target.index_predicate {
        sink.text(" WHERE ");
        crate::render::render_expr::<D>(predicate, sink);
    }
    match &clause.action {
        ConflictAction::DoNothing => sink.text(" DO NOTHING"),
        ConflictAction::DoUpdate(sets) => {
            sink.text(" DO UPDATE SET ");
            sets.render_into(sink);
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

    /// `INSERT INTO t (..) SELECT ..`: the rows a query produces, checked
    /// against the target's own row by `row::SameShape`, the same one
    /// comparison a `UNION` branch and a CTE body go through.
    ///
    /// The query fills every column the target lets a statement write: all
    /// of them but the generated ones, which the database writes itself and
    /// refuses a value for. `SameShape` compares name and type cell by
    /// cell, so the source's columns must be spelled and typed as the
    /// target's. SQL would widen an `INTEGER` into a `BIGINT` and take a
    /// NOT NULL value for a nullable column, and neither is accepted here.
    /// A source column under another name takes a `label!{}` one.
    ///
    /// **Known limitations**: the source is a `Select`, so a `SetOp`
    /// (`UNION`) or a `DynSelect` cannot be one; a one-column target still
    /// needs a one-tuple (`select((t::only,))`), since a bare selection is
    /// a value rather than a row.
    pub fn select<Scope, Sel, SelIdx, TgtIdx>(
        self,
        query: &crate::select::Select<D, Scope, Sel>,
    ) -> InsertSelect<D, T>
    where
        D: Dialect,
        T: WrittenColumns,
        T::Columns: crate::select::ColumnList<WrittenTable<T>, TgtIdx>,
        TargetRow<T, TgtIdx>: crate::row::ColumnNames,
        Sel: crate::select::Selection<Scope, SelIdx>,
        Sel::Output: crate::row::SameShape<crate::row::Row<TargetRow<T, TgtIdx>>>,
    {
        InsertSelect {
            // Read off the very row the query was checked against, the way
            // a `WITH` header is: one fact rather than two that could name
            // different columns.
            header: <TargetRow<T, TgtIdx> as crate::row::ColumnNames>::names(),
            body: query.fragment::<SelIdx>(),
            _marker: PhantomData,
        }
    }

    /// Every row of a collection at once: the shape a bulk import has,
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

/// The columns of a table an `INSERT` may name: all of them but the
/// generated ones, which the database writes itself and refuses a value
/// for. Emitted by `#[derive(Table)]` beside `select::AllColumns`, which is
/// the other list: what a `SELECT` of the whole table reads.
#[doc(hidden)]
pub trait WrittenColumns {
    /// `Cons<Column<C>, ..>`, in the schema's own order.
    type Columns;
}

/// The row an `INSERT INTO t (..) SELECT ..` has to be handed: the target's
/// writable columns, read in the one-table scope a write statement has.
type TargetRow<T, Idx> = <<T as WrittenColumns>::Columns as crate::select::ColumnList<
    WrittenTable<T>,
    Idx,
>>::Fields<crate::row::RowNil>;

/// `INSERT INTO t (..) SELECT ..`: rows a query produces rather than rows
/// a caller holds. From [`InsertSeed::select`].
///
/// The header is the target's own writable columns, so the two sides line
/// up by the target's order rather than by whatever order its
/// `CREATE TABLE` happened to use. The body is a `render::Fragment`,
/// rendered before the statement knows how many parameters precede it, for
/// the reason a CTE body is one.
///
/// **Known limitation**: no `ON CONFLICT` on this shape, and no column
/// subset. The query fills every column the target lets one write.
pub struct InsertSelect<D, T> {
    header: Vec<&'static str>,
    body: crate::render::Fragment,
    _marker: PhantomData<fn() -> (D, T)>,
}

impl<D: Dialect, T: Table> crate::statement::private::Sealed for InsertSelect<D, T> {}

impl<D: Dialect, T: Table> Statement for InsertSelect<D, T> {
    type Dialect = D;
    type Table = T;
    fn render_into(&self, sink: &mut dyn Sink) {
        sink.text("INSERT INTO ");
        render_ident::<D>(sink, T::NAME);
        sink.text(" (");
        for (i, name) in self.header.iter().enumerate() {
            if i > 0 {
                sink.text(", ");
            }
            render_ident::<D>(sink, name);
        }
        sink.text(") ");
        self.body.splice_into(sink);
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
    sink: &mut dyn Sink,
    rows: &[Vec<InsertValue>],
    on_conflict: &Option<ConflictClause<D, R::Table>>,
) {
    sink.text("INSERT INTO ");
    render_ident::<D>(sink, <R::Table as Table>::NAME);

    // Header and values come from one chain, so a row has exactly one cell
    // per column named: no reconciliation, nothing to drop.
    let mut header = Vec::new();
    <R::Values as InsertValues>::push_names(&mut header);

    // A table whose every column is generated leaves nothing to name, and
    // an empty column list is a syntax error in two of the three dialects.
    // One such row is all SQL can express, which is why `values`/`values_all`
    // take `Insertable`.
    if header.is_empty() {
        sink.text(D::INSERT_NO_COLUMNS);
        if let Some(clause) = on_conflict {
            render_conflict_clause::<D, _>(clause, sink);
        }
        return;
    }

    sink.text(" (");
    for (i, name) in header.iter().enumerate() {
        if i > 0 {
            sink.text(", ");
        }
        render_ident::<D>(sink, name);
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
        render_conflict_clause::<D, _>(clause, sink);
    }
}

pub struct Insert<D, R: InsertRow> {
    rows: Vec<Vec<InsertValue>>,
    on_conflict: Option<ConflictClause<D, R::Table>>,
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
    /// `ON CONFLICT (..) DO NOTHING`. Pass [`partial_index`] where the
    /// index to infer is a partial one.
    pub fn on_conflict_do_nothing(mut self, target: impl ConflictTarget<R::Table>) -> Self
    where
        D: SupportsOnConflict,
    {
        self.on_conflict = Some(ConflictClause {
            target: target.into_target(),
            action: ConflictAction::DoNothing,
        });
        self
    }

    /// `ON CONFLICT (..) DO UPDATE SET ..`. Takes the [`Assignments`] an
    /// `UPDATE` sets, or a [`ConflictUpdate`] where an assignment names the
    /// row the `INSERT` proposed. Pass [`partial_index`] where the index to
    /// infer is a partial one. A [`ConflictUpdate`] is also where the
    /// `WHERE` that decides whether the update fires at all lives.
    pub fn on_conflict_do_update(
        mut self,
        target: impl ConflictTarget<R::Table>,
        set: impl Into<ConflictUpdate<D, R::Table>>,
    ) -> Self
    where
        D: SupportsOnConflict,
    {
        self.on_conflict = Some(ConflictClause {
            target: target.into_target(),
            action: ConflictAction::DoUpdate(set.into()),
        });
        self
    }
}

impl<D: Dialect, R: InsertRow> crate::statement::private::Sealed for Insert<D, R> {}

impl<D: Dialect, R: InsertRow> Statement for Insert<D, R> {
    type Dialect = D;
    type Table = R::Table;
    fn render_into(&self, sink: &mut dyn Sink) {
        render_values_clause::<D, R>(sink, &self.rows, &self.on_conflict);
    }
}

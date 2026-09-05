//! The `SELECT` builder: `select(..).from(..).join(..).filter(..)
//! .order_by(..).limit(..)`, in SQL keyword order, with tables passed as
//! arguments rather than by turbofish.

use std::marker::PhantomData;

use crate::cte::Cte;
use crate::dialect::{Dialect, SupportsFullOuterJoin, SupportsRightJoin};
use crate::expr::{BoolLike, Comparable, Expr, ExprKind, IntoExpr, Value};
use crate::render::{
    Fragment, FragmentSink, QuerySink, SelectItem, Sink, render_and_list, render_expr,
    render_expr_list, render_order_by, render_select_list,
};
use crate::scope::{
    BaseTable, Concat, Cons, MapNullable, MaybeNull, Nil, NotNull, ScopeTables, Superset, Table,
    TableSlot,
};

mod dyn_select;
mod prepared;
mod selection;
mod set_op;

pub use crate::expr::SortDir;
pub use dyn_select::{CannotFilterAfterErase, DynSelect};
pub use prepared::{Prepared, PreparedParams, Total, UnresolvedPlaceholder};
pub use selection::{
    All, AllColumns, ColumnList, RowField, SelectableSealed, Selection, SelectionPart, SingleColumn,
};
pub use set_op::SetOp;

/// One `name AS (body)` binding, carried in by the `Cte` a query was
/// entered through. Opaque outside this crate: `JoinSource::binding` is the
/// only way to make one, and it takes the name and columns from the
/// marker's `CteShape` rather than from anything a caller can vary.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct CteDef {
    name: &'static str,
    column_names: Vec<&'static str>,
    body: Fragment,
}

impl CteDef {
    pub(crate) fn new(name: &'static str, column_names: Vec<&'static str>, body: Fragment) -> Self {
        CteDef {
            name,
            column_names,
            body,
        }
    }
}

#[derive(Debug, Clone)]
enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
}

#[derive(Debug, Clone)]
struct JoinClause {
    kind: JoinKind,
    table: &'static str,
    on: ExprKind,
}

/// An order-by key: a column/expression tagged with a direction. Built via
/// `.asc()`/`.desc()` on any column or expression (see `OrderExt` below),
/// so `.order_by(users::created_at.desc())` reads as one argument, not two.
pub struct OrderKey<Req> {
    kind: ExprKind,
    dir: SortDir,
    _marker: PhantomData<fn() -> Req>,
}

// Hand-written for the reason `Expr`'s is: `#[derive(Clone)]` would ask the
// phantom `Req` to be `Clone`, which no scope list is.
impl<Req> Clone for OrderKey<Req> {
    fn clone(&self) -> Self {
        OrderKey {
            kind: self.kind.clone(),
            dir: self.dir,
            _marker: PhantomData,
        }
    }
}

impl<Req> OrderKey<Req> {
    pub(crate) fn into_parts(self) -> (ExprKind, SortDir) {
        (self.kind, self.dir)
    }
}

pub trait OrderExt: IntoExpr + Sized {
    fn asc(self) -> OrderKey<Self::Req> {
        OrderKey {
            kind: self.into_expr().kind,
            dir: SortDir::Asc,
            _marker: PhantomData,
        }
    }
    /// The direction as a value, for a sort order that arrives at runtime —
    /// `?dir=desc` — instead of an N-way match over `.asc()`/`.desc()`.
    fn sort(self, dir: SortDir) -> OrderKey<Self::Req> {
        OrderKey {
            kind: self.into_expr().kind,
            dir,
            _marker: PhantomData,
        }
    }
    fn desc(self) -> OrderKey<Self::Req> {
        OrderKey {
            kind: self.into_expr().kind,
            dir: SortDir::Desc,
            _marker: PhantomData,
        }
    }
}
impl<T: IntoExpr> OrderExt for T {}

/// Something a query can select from or join to. A schema table brings
/// nothing with it; a `Cte` brings its `WITH` binding, so attaching that
/// binding and putting the pseudo-table in scope stay one act — while every
/// join kind, and `correlated`, work on both without being written twice.
pub trait JoinSource<D>: join_source::Sealed {
    /// The table this source contributes to the scope.
    type Table: Table;
    /// The `WITH` binding this source carries into the statement — `None`
    /// for a schema table, which is already there.
    #[doc(hidden)]
    fn binding(self) -> Option<CteDef>;
}

mod join_source {
    /// Sealed the way `dialect::Dialect` is: what a query can read from is
    /// a closed set of two, a schema table and a bound CTE.
    pub trait Sealed {}
    impl<T: super::BaseTable> Sealed for T {}
    impl<D, Marker> Sealed for crate::cte::Cte<D, Marker> {}
}

impl<D, T: BaseTable> JoinSource<D> for T {
    type Table = T;
    fn binding(self) -> Option<CteDef> {
        None
    }
}

impl<D, Marker: crate::cte::CteShape> JoinSource<D> for Cte<D, Marker> {
    type Table = Marker;
    fn binding(self) -> Option<CteDef> {
        Some(CteDef::new(
            <Marker as Table>::NAME,
            <Marker::Row as crate::row::ColumnNames>::names(),
            self.into_body(),
        ))
    }
}

/// Something a query can be ordered by: an `OrderKey` whose tables this
/// scope contains, or a `SortKey` already discharged against it — the same
/// pair `Condition` makes for `.filter`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a sort key",
    label = "a column or expression with `.asc()`/`.desc()`/`.sort(dir)` on it, or a `sort_key(..)`",
    note = "a `SortKey` also has to have been discharged against *this* scope — a scope lists its tables most-recently-joined first, so two that look alike can still differ in order"
)]
pub trait SortBy<Scope, Idxs> {
    #[doc(hidden)]
    fn into_sort_key(self) -> SortKey<Scope>;
}

impl<Scope: Superset<Req, Idxs>, Req, Idxs> SortBy<Scope, Idxs> for OrderKey<Req> {
    fn into_sort_key(self) -> SortKey<Scope> {
        SortKey {
            kind: self.kind,
            dir: self.dir,
            _marker: PhantomData,
        }
    }
}

impl<Scope> SortBy<Scope, ()> for SortKey<Scope> {
    fn into_sort_key(self) -> SortKey<Scope> {
        self
    }
}

/// The same for `GROUP BY`: an expression this scope covers, or a
/// `grouping(..)` already discharged against it.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a grouping key",
    label = "a column or expression, or a `grouping(..)`",
    note = "a `Grouping` also has to have been discharged against *this* scope — a scope lists its tables most-recently-joined first, so two that look alike can still differ in order"
)]
pub trait GroupBy<Scope, Idxs> {
    #[doc(hidden)]
    fn into_grouping(self) -> Grouping<Scope>;
}

impl<Scope: Superset<Req, Idxs>, Req, Idxs, T: IntoExpr<Req = Req>> GroupBy<Scope, Idxs> for T {
    fn into_grouping(self) -> Grouping<Scope> {
        Grouping {
            kind: self.into_expr().kind,
            _marker: PhantomData,
        }
    }
}

impl<Scope> GroupBy<Scope, ()> for Grouping<Scope> {
    fn into_grouping(self) -> Grouping<Scope> {
        self
    }
}

/// A sort key whose scope requirement has already been discharged, so a
/// runtime-length collection of them can be built and passed around — the
/// `?sort=email,-placed_on` case. `select::sort_key` is to `.order_by_all`
/// what `predicate` is to `.filter_all`.
pub struct SortKey<Scope> {
    kind: ExprKind,
    dir: SortDir,
    _marker: PhantomData<fn() -> Scope>,
}

impl<Scope> Clone for SortKey<Scope> {
    fn clone(&self) -> Self {
        SortKey {
            kind: self.kind.clone(),
            dir: self.dir,
            _marker: PhantomData,
        }
    }
}

/// Discharges a sort key's scope requirement. `Scope` is inferred from the
/// query the keys are eventually given to. Takes whatever `.order_by` takes,
/// as `predicate` takes whatever `.filter` does — so a helper generic over
/// `SortBy` can discharge without knowing which of the two it was handed.
pub fn sort_key<Scope, Idxs, K: SortBy<Scope, Idxs>>(key: K) -> SortKey<Scope> {
    key.into_sort_key()
}

/// A grouping key with its scope requirement discharged — `predicate`'s
/// counterpart for `GROUP BY`.
pub struct Grouping<Scope> {
    kind: ExprKind,
    _marker: PhantomData<fn() -> Scope>,
}

impl<Scope> Clone for Grouping<Scope> {
    fn clone(&self) -> Self {
        Grouping {
            kind: self.kind.clone(),
            _marker: PhantomData,
        }
    }
}

/// Discharges a grouping key's scope requirement, taking whatever
/// `.group_by` takes — the same shape `predicate` and `sort_key` have.
pub fn grouping<Scope, Idxs, K: GroupBy<Scope, Idxs>>(key: K) -> Grouping<Scope> {
    key.into_grouping()
}

/// Something a query can be filtered by: an `Expr` whose tables this scope
/// contains, or a `Predicate` already discharged against it. One `.filter`
/// for both, so which one a condition happens to be doesn't change how it is
/// applied.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a condition",
    label = "a comparison (`.eq(..)`, `.gt(..)`, `.is_null()`), an `any_of`/`all_of` of them, a `sql!` fragment of type `Bool`, a `predicate(..)`, or an `EXISTS`/`.contains(..)` of a subquery in *this* dialect",
    note = "a `Predicate` also has to have been discharged against *this* scope — a scope lists its tables most-recently-joined first, so two that look alike can still differ in order"
)]
pub trait Condition<D, Scope, Idxs> {
    /// Discharged against this scope, which for an `Expr` is where its
    /// `Superset` proof is spent and for a `Predicate` already happened.
    #[doc(hidden)]
    fn into_predicate(self) -> Predicate<D, Scope>;
}

impl<D, Scope: Superset<Req, Idxs>, Req, Idxs, T: IntoExpr<Req = Req>> Condition<D, Scope, Idxs>
    for T
where
    T::Sql: BoolLike,
{
    fn into_predicate(self) -> Predicate<D, Scope> {
        Predicate {
            kind: self.into_expr().kind,
            _marker: PhantomData,
        }
    }
}

impl<D, Scope> Condition<D, Scope, ()> for Predicate<D, Scope> {
    fn into_predicate(self) -> Predicate<D, Scope> {
        self
    }
}

/// `EXISTS (<subquery>)`. Not an `Expr`, because unlike every other
/// expression this one is dialect-pinned: the subquery it holds was
/// capability-checked against its own dialect, and any CTE it binds is
/// already rendered in that dialect. It is therefore a condition and only a
/// condition — `.filter(..)` it, or `predicate(..)` it into a collection,
/// onto a query of the same dialect. A `Predicate` carries `D` for this
/// reason: discharging a condition gives up the tables it named, never the
/// dialect it was built for.
pub struct Exists<D, Req> {
    kind: ExprKind,
    _marker: PhantomData<fn() -> (D, Req)>,
}

// Hand-written for the reason `Expr`'s is: `#[derive(Clone)]` would ask the
// phantom tags to be `Clone`.
impl<D, Req> Clone for Exists<D, Req> {
    fn clone(&self) -> Self {
        Exists {
            kind: self.kind.clone(),
            _marker: PhantomData,
        }
    }
}

impl<D, Scope: Superset<Req, Idxs>, Req, Idxs> Condition<D, Scope, Idxs> for Exists<D, Req> {
    fn into_predicate(self) -> Predicate<D, Scope> {
        Predicate {
            kind: self.kind,
            _marker: PhantomData,
        }
    }
}

/// `lhs IN (<subquery>)` / `lhs NOT IN (<subquery>)`. Not an `Expr`, for the
/// same reason `Exists` isn't: the subquery it holds was capability-checked
/// against its own dialect. `.filter(..)` it, or `predicate(..)` it into a
/// collection, onto a query of the same dialect.
pub struct InSubquery<D, Req> {
    kind: ExprKind,
    _marker: PhantomData<fn() -> (D, Req)>,
}

impl<D, Req> Clone for InSubquery<D, Req> {
    fn clone(&self) -> Self {
        InSubquery {
            kind: self.kind.clone(),
            _marker: PhantomData,
        }
    }
}

impl<D, Scope: Superset<Req, Idxs>, Req, Idxs> Condition<D, Scope, Idxs> for InSubquery<D, Req> {
    fn into_predicate(self) -> Predicate<D, Scope> {
        Predicate {
            kind: self.kind,
            _marker: PhantomData,
        }
    }
}

/// A condition whose scope requirement has already been discharged, so a
/// runtime-length collection of them can be built and passed around. An
/// `Expr` carries the tables it references in its type, which is what makes
/// `vec![users_cond, orders_cond]` fail to unify; `predicate` trades that
/// tag for a proof against one concrete scope.
pub struct Predicate<D, Scope> {
    kind: ExprKind,
    _marker: PhantomData<fn() -> (D, Scope)>,
}

// Hand-written for the reason `Expr`'s is: `#[derive(Clone)]` would ask a
// phantom `Scope` to be `Clone`.
impl<D, Scope> Clone for Predicate<D, Scope> {
    fn clone(&self) -> Self {
        Predicate {
            kind: self.kind.clone(),
            _marker: PhantomData,
        }
    }
}

impl<D, Scope> Predicate<D, Scope> {
    /// True when any of them is. `expr::any_of` combines conditions that
    /// reference the same tables; this one combines conditions whose scope
    /// requirement is already discharged, which is what lets a search form
    /// OR together conditions from different tables.
    pub fn any_of(preds: impl IntoIterator<Item = Predicate<D, Scope>>) -> Self {
        Predicate::combine(preds, false)
    }

    /// True when all of them are — the AND to `any_of`'s OR, so a group of
    /// them can be nested inside one.
    pub fn all_of(preds: impl IntoIterator<Item = Predicate<D, Scope>>) -> Self {
        Predicate::combine(preds, true)
    }

    fn combine(preds: impl IntoIterator<Item = Predicate<D, Scope>>, all: bool) -> Self {
        Predicate {
            kind: crate::expr::fold_conditions(preds.into_iter().map(Predicate::into_kind), all),
            _marker: PhantomData,
        }
    }

    pub(crate) fn into_kind(self) -> ExprKind {
        self.kind
    }
}

/// Discharges a condition's scope requirement, so a runtime-length
/// collection of them can be built and passed around. `Scope` and `D` are
/// inferred from the query the resulting predicates are eventually given to.
pub fn predicate<D, Scope, Idxs, C: Condition<D, Scope, Idxs>>(cond: C) -> Predicate<D, Scope> {
    cond.into_predicate()
}

/// Holds just the `SELECT` list until `.from(..)` supplies the first table
/// and therefore the query's initial `Scope`. Splitting this out (rather
/// than requiring `Scope` be known at `.select(..)` time) is what lets the
/// builder read in `SELECT -> FROM -> ...` order while still validating
/// every selected column against the *final* scope only once, at the
/// query's terminal method (`.to_sql(Postgres)`/`.load()`).
pub struct SelectSeed<Sel> {
    selection: Sel,
}

pub fn select<Sel>(selection: Sel) -> SelectSeed<Sel> {
    SelectSeed { selection }
}

impl<Sel> SelectSeed<Sel> {
    /// `source` is a value, not a turbofish — a schema table's zero-sized
    /// token (`users::Table`) or a `cte::with(..)` binding. A CTE brings its
    /// `WITH` clause along, so it cannot be selected from unbound.
    pub fn from<D, S: JoinSource<D>>(
        self,
        source: S,
    ) -> Select<D, Cons<TableSlot<S::Table, NotNull>, Nil>, Sel> {
        let mut body = SelectBody::new(<S::Table as Table>::NAME);
        body.bind(source);
        Select {
            body,
            selection: self.selection,
            _marker: PhantomData,
        }
    }
}

/// Every clause of a `SELECT` except the selection list, which `Select`
/// keeps typed and `DynSelect` keeps rendered. Held whole by both, so a new
/// clause is added here once instead of being threaded through each of them
/// and through rendering by hand.
#[derive(Debug, Clone)]
pub(crate) struct SelectBody {
    ctes: Vec<CteDef>,
    distinct: bool,
    from_table: &'static str,
    joins: Vec<JoinClause>,
    wheres: Vec<ExprKind>,
    order_by: Vec<(ExprKind, SortDir)>,
    group_by: Vec<ExprKind>,
    having: Vec<ExprKind>,
    limit: Option<RowCount>,
    offset: Option<RowCount>,
}

impl SelectBody {
    fn new(from_table: &'static str) -> Self {
        SelectBody {
            ctes: Vec::new(),
            distinct: false,
            from_table,
            joins: Vec::new(),
            wheres: Vec::new(),
            order_by: Vec::new(),
            group_by: Vec::new(),
            having: Vec::new(),
            limit: None,
            offset: None,
        }
    }

    /// `SELECT count(*)` over this body with its paging dropped: a total
    /// counts the rows that match, not the page being shown.
    ///
    /// A query whose rows aren't one per matching row is counted by
    /// wrapping it, selection and all, since what a page of it would show is
    /// what has to be counted. Stated as what may be left unwrapped — plain
    /// columns, no `GROUP BY`/`HAVING`/`DISTINCT` — rather than as a list of
    /// what may not: an aggregate collapses the rows too, and `sql!` can
    /// hold anything at all.
    fn count_sql<D: Dialect>(&self, selection: &[SelectItem]) -> (String, Vec<Value>) {
        let mut body = self.clone();
        body.order_by.clear();
        body.limit = None;
        body.offset = None;
        let one_row_each = body.group_by.is_empty()
            && body.having.is_empty()
            && !body.distinct
            && selection
                .iter()
                .all(|item| matches!(item.kind, ExprKind::Column { .. }));

        let mut sink = QuerySink::<D>::new();
        if one_row_each {
            body.render_into::<D>(&[crate::expr::count_item()], &mut sink);
            return sink.finish();
        }
        crate::render::render_count_wrapped::<D>(&mut sink, |sink| {
            body.render_into::<D>(selection, sink)
        });
        sink.finish()
    }

    /// Attaches whatever `WITH` binding a join source carries before its
    /// table is named in a FROM or JOIN clause.
    fn bind<D, S: JoinSource<D>>(&mut self, source: S) {
        self.ctes.extend(source.binding());
    }
}

/// A `SELECT`. `Outer` is the scope this query was built *against* — `Nil`
/// for a query of its own, and the outer query's scope for one started by
/// `.correlated(..)`, which is what lets `EXISTS` report the outer tables it
/// references. Defaulted, so a query that isn't a subquery never spells it.
pub struct Select<D, Scope, Sel, Outer = Nil> {
    body: SelectBody,
    selection: Sel,
    _marker: PhantomData<fn() -> (D, Scope, Outer)>,
}

// Cloning is what lets one built-up query serve both a count and a page.
impl<D, Scope, Sel: Clone, Outer> Clone for Select<D, Scope, Sel, Outer> {
    fn clone(&self) -> Self {
        Select {
            body: self.body.clone(),
            selection: self.selection.clone(),
            _marker: PhantomData,
        }
    }
}

impl<D, Scope, Sel, Outer> Select<D, Scope, Sel, Outer> {
    fn retype<NewScope>(self) -> Select<D, NewScope, Sel, Outer> {
        Select {
            body: self.body,
            selection: self.selection,
            _marker: PhantomData,
        }
    }

    /// Swaps the selection list, keeping every clause. With `Clone`, this is
    /// how one built-up query serves both a count and a page.
    pub fn reselect<NewSel>(self, selection: NewSel) -> Select<D, Scope, NewSel, Outer> {
        Select {
            body: self.body,
            selection,
            _marker: PhantomData,
        }
    }

    /// AND-folded, and callable any number of times — conditionally, in a
    /// loop, from a helper — without changing `Self`'s type, so the most
    /// common kind of dynamic query needs no escape hatch.
    pub fn filter<C: Condition<D, Scope, Idxs>, Idxs>(mut self, cond: C) -> Self {
        self.body.wheres.push(cond.into_predicate().into_kind());
        self
    }

    /// AND-folds a runtime-length collection of already-discharged
    /// conditions — the shape a search form has, where the conditions come
    /// from different tables and so can't share one `Expr` type.
    pub fn filter_all(mut self, conds: impl IntoIterator<Item = Predicate<D, Scope>>) -> Self {
        self.body
            .wheres
            .extend(conds.into_iter().map(Predicate::into_kind));
        self
    }

    pub fn order_by<K: SortBy<Scope, Idxs>, Idxs>(mut self, key: K) -> Self {
        let key = key.into_sort_key();
        self.body.order_by.push((key.kind, key.dir));
        self
    }

    /// Appends a runtime-length collection of already-discharged sort keys
    /// — the shape a `?sort=` parameter has, where the keys name different
    /// tables and so can't share one `OrderKey` type.
    pub fn order_by_all(mut self, keys: impl IntoIterator<Item = SortKey<Scope>>) -> Self {
        self.body
            .order_by
            .extend(keys.into_iter().map(|k| (k.kind, k.dir)));
        self
    }

    /// `SELECT DISTINCT`: one row per distinct selected tuple. The natural
    /// answer to a one-to-many join that repeats its left side, and unlike a
    /// `GROUP BY` of the whole selection it doesn't have to be restated when
    /// the selection changes. Idempotent — a query is distinct or it isn't.
    ///
    /// **Known limitation**: Postgres requires a `SELECT DISTINCT`'s sort
    /// keys to be in its selection, and nothing here relates the two — the
    /// same gap `GROUP BY` has. Sorting a distinct query by a column it
    /// doesn't select renders SQL the database rejects, and `count_sql`
    /// won't show it, since a total drops the `ORDER BY`. Relating them
    /// would mean carrying "is distinct" in `Select`'s type and taking sort
    /// keys by identity (`SetOp::order_by_column`'s shape) — a type
    /// parameter through every builder signature for one clause.
    pub fn distinct(mut self) -> Self {
        self.body.distinct = true;
        self
    }

    /// Appends one grouping key; callable multiple times like `.filter()`
    /// (each call adds a column to the `GROUP BY` list, it doesn't replace
    /// it), for the same "dynamic composition without a type change" reason.
    pub fn group_by<K: GroupBy<Scope, Idxs>, Idxs>(mut self, key: K) -> Self {
        self.body.group_by.push(key.into_grouping().kind);
        self
    }

    /// The same for a runtime-length collection of discharged grouping
    /// keys, as `order_by_all` is to `order_by`.
    pub fn group_by_all(mut self, keys: impl IntoIterator<Item = Grouping<Scope>>) -> Self {
        self.body.group_by.extend(keys.into_iter().map(|g| g.kind));
        self
    }

    /// A `WHERE`-shaped filter applied after grouping (aggregate
    /// conditions) — AND-folded across calls exactly like `.filter()`.
    pub fn having<C: Condition<D, Scope, Idxs>, Idxs>(mut self, cond: C) -> Self {
        self.body.having.push(cond.into_predicate().into_kind());
        self
    }

    /// The same for a runtime-length collection of discharged conditions,
    /// as `filter_all` is to `filter`.
    pub fn having_all(mut self, conds: impl IntoIterator<Item = Predicate<D, Scope>>) -> Self {
        self.body
            .having
            .extend(conds.into_iter().map(Predicate::into_kind));
        self
    }

    pub fn limit(mut self, n: impl IntoRowCount) -> Self {
        self.body.limit = Some(n.into_row_count());
        self
    }

    pub fn offset(mut self, n: impl IntoRowCount) -> Self {
        self.body.offset = Some(n.into_row_count());
        self
    }

    /// The joined table is in scope for the `ON` condition, and so is
    /// everything already joined — the scope the condition is discharged
    /// against is the one the join produces, not the one it started from.
    pub fn inner_join<S: JoinSource<D>, C, Idxs>(
        mut self,
        source: S,
        on: C,
    ) -> Select<D, Cons<TableSlot<S::Table, NotNull>, Scope>, Sel, Outer>
    where
        C: Condition<D, Cons<TableSlot<S::Table, NotNull>, Scope>, Idxs>,
    {
        self.body.bind(source);
        self.body.joins.push(JoinClause {
            kind: JoinKind::Inner,
            table: <S::Table as Table>::NAME,
            on: on.into_predicate().into_kind(),
        });
        self.retype()
    }

    pub fn left_join<S: JoinSource<D>, C, Idxs>(
        mut self,
        source: S,
        on: C,
    ) -> Select<D, Cons<TableSlot<S::Table, MaybeNull>, Scope>, Sel, Outer>
    where
        C: Condition<D, Cons<TableSlot<S::Table, MaybeNull>, Scope>, Idxs>,
    {
        self.body.bind(source);
        self.body.joins.push(JoinClause {
            kind: JoinKind::Left,
            table: <S::Table as Table>::NAME,
            on: on.into_predicate().into_kind(),
        });
        self.retype()
    }

    /// RIGHT JOIN retroactively flips every already-joined table to
    /// nullable (`MapNullable`) before adding the new, guaranteed-present
    /// table, mirroring Drizzle's `AppendToNullabilityMap` rule.
    pub fn right_join<S: JoinSource<D>, C, Idxs>(
        mut self,
        source: S,
        on: C,
    ) -> Select<D, Cons<TableSlot<S::Table, NotNull>, Scope::Output>, Sel, Outer>
    where
        D: SupportsRightJoin,
        Scope: MapNullable,
        C: Condition<D, Cons<TableSlot<S::Table, NotNull>, Scope::Output>, Idxs>,
    {
        self.body.bind(source);
        self.body.joins.push(JoinClause {
            kind: JoinKind::Right,
            table: <S::Table as Table>::NAME,
            on: on.into_predicate().into_kind(),
        });
        self.retype()
    }

    pub fn full_join<S: JoinSource<D>, C, Idxs>(
        mut self,
        source: S,
        on: C,
    ) -> Select<D, Cons<TableSlot<S::Table, MaybeNull>, Scope::Output>, Sel, Outer>
    where
        D: SupportsFullOuterJoin,
        Scope: MapNullable,
        C: Condition<D, Cons<TableSlot<S::Table, MaybeNull>, Scope::Output>, Idxs>,
    {
        self.body.bind(source);
        self.body.joins.push(JoinClause {
            kind: JoinKind::Full,
            table: <S::Table as Table>::NAME,
            on: on.into_predicate().into_kind(),
        });
        self.retype()
    }
}

/// The terminal methods, and so only for a query of its own: a subquery
/// (`Outer != Nil`) references its outer query's tables, and rendering one
/// on its own would name tables that aren't in its `FROM`. It reaches SQL
/// through `exists`/`not_exists` instead.
impl<D: Dialect, Scope, Sel> Select<D, Scope, Sel> {
    /// The terminal step, and the *only* point each selected column's
    /// scope-membership is checked — proven as a side effect of
    /// `Sel: Selection<Scope, Idx>` type-checking at all.
    ///
    /// The dialect is an argument rather than a turbofish, so a query that
    /// is rendered instead of executed says which SQL it wants in the one
    /// place that decides — and everything before it infers, the way a
    /// table or a column does.
    pub fn to_sql<Idx>(&self, _dialect: D) -> (String, Vec<Value>)
    where
        Sel: Selection<Scope, Idx>,
    {
        self.render_as::<Idx>()
    }

    /// How many rows this query would return, ignoring its
    /// `ORDER BY`/`LIMIT`/`OFFSET` — a total is about what matches, not about
    /// the page being shown. `reselect(count())` keeps them, which is what
    /// makes it the wrong tool for a paginated total.
    ///
    /// A grouped query counts its *groups*, since that is what a page of it
    /// would show, so the body becomes a subquery rather than having its
    /// `GROUP BY` dropped or kept.
    pub fn count_sql<Idx>(&self, _dialect: D) -> (String, Vec<Value>)
    where
        Sel: Selection<Scope, Idx>,
    {
        self.body.count_sql::<D>(&self.selection.items())
    }
}

impl<D: Dialect, Scope, Sel> Select<D, Scope, Sel> {
    /// This query as an embeddable `Fragment`: an `EXISTS (..)` subquery, a
    /// CTE body, or a set-operation branch. The only way to produce one, so
    /// no caller has to remember that an embedded query renders with `?`
    /// placeholders rather than `D`'s own style. `Outer = Nil` like the
    /// other terminals: a correlated subquery names tables that aren't in
    /// its own `FROM`, and reaches SQL through `EXISTS` instead.
    pub(crate) fn fragment<Idx>(&self) -> Fragment
    where
        Sel: Selection<Scope, Idx>,
    {
        let mut sink = FragmentSink::new();
        self.body
            .render_into::<D>(&self.selection.items(), &mut sink);
        sink.finish()
    }

    fn render_as<Idx>(&self) -> (String, Vec<Value>)
    where
        Sel: Selection<Scope, Idx>,
    {
        let mut sink = QuerySink::<D>::new();
        self.body
            .render_into::<D>(&self.selection.items(), &mut sink);
        sink.finish()
    }
}

impl SelectBody {
    pub(crate) fn render_into<D: Dialect>(&self, selection: &[SelectItem], sink: &mut dyn Sink) {
        let SelectBody {
            ctes,
            distinct,
            from_table,
            joins,
            wheres,
            order_by,
            group_by,
            having,
            limit,
            offset,
        } = self;

        if !ctes.is_empty() {
            sink.text("WITH ");
            for (i, cte) in ctes.iter().enumerate() {
                if i > 0 {
                    sink.text(", ");
                }
                crate::render::render_ident::<D>(sink, cte.name);
                sink.text(" (");
                for (i, col) in cte.column_names.iter().enumerate() {
                    if i > 0 {
                        sink.text(", ");
                    }
                    crate::render::render_ident::<D>(sink, col);
                }
                sink.text(") AS (");
                cte.body.splice_into(sink);
                sink.ch(')');
            }
            sink.ch(' ');
        }
        sink.text("SELECT ");
        if *distinct {
            sink.text("DISTINCT ");
        }

        render_select_list::<D>(selection, sink);

        sink.text(" FROM ");
        crate::render::render_ident::<D>(sink, from_table);

        for j in joins {
            sink.ch(' ');
            sink.text(match j.kind {
                JoinKind::Inner => "INNER JOIN",
                JoinKind::Left => "LEFT JOIN",
                JoinKind::Right => "RIGHT JOIN",
                JoinKind::Full => "FULL JOIN",
            });
            sink.ch(' ');
            crate::render::render_ident::<D>(sink, j.table);
            sink.text(" ON ");
            render_expr::<D>(&j.on, sink);
        }

        render_and_list::<D>(sink, " WHERE ", wheres);

        render_expr_list::<D>(sink, " GROUP BY ", group_by);
        render_and_list::<D>(sink, " HAVING ", having);
        render_order_by::<D>(sink, " ORDER BY ", order_by);
        render_limit_offset::<D>(sink, limit.as_ref(), offset.as_ref());
    }
}

/// Starts a correlated subquery against a scope, rather than against a
/// query — so `UPDATE`/`DELETE`, whose scope is the one table they write,
/// reach the same `EXISTS` a `SELECT` does without conjuring a `Select`
/// they don't otherwise need.
pub(crate) fn correlated_with<D, Scope, S: JoinSource<D>, InnerSel>(
    source: S,
    selection: InnerSel,
) -> Select<D, Cons<TableSlot<S::Table, NotNull>, Scope>, InnerSel, Scope> {
    let mut body = SelectBody::new(<S::Table as Table>::NAME);
    body.bind(source);
    Select {
        body,
        selection,
        _marker: PhantomData,
    }
}

impl<D, Scope, Sel, Outer> Select<D, Scope, Sel, Outer> {
    /// Starts a correlated subquery: a fresh `SELECT` whose scope is
    /// `Cons<TableSlot<T, NotNull>, Scope>` — the new table, prepended onto
    /// *this* (outer) query's entire scope. Because `Find`/`Superset` walk
    /// the whole flat cons-list regardless of where it came from, the
    /// subquery's `.filter()` can reference both its own new table's
    /// columns and any outer column already in `Scope`, with no special
    /// casing: growing the scope works the same whether the new table came
    /// from a join or from a subquery's `FROM`.
    ///
    /// The result is an ordinary `Select` — every clause it takes is the
    /// one `Select` already has — carrying this query's scope as its
    /// `Outer`, which is what `exists` reports.
    pub fn correlated<S: JoinSource<D>, InnerSel>(
        &self,
        source: S,
        selection: InnerSel,
    ) -> Select<D, Cons<TableSlot<S::Table, NotNull>, Scope>, InnerSel, Scope> {
        correlated_with(source, selection)
    }
}

impl<D: Dialect, Scope, Sel, Outer: ScopeTables> Select<D, Scope, Sel, Outer> {
    /// `EXISTS (<this query>)`, tagged with the outer tables it references
    /// so it can only be filtered onto a query that has them in scope. Its
    /// own column references were already checked against `Scope` when it
    /// was built. On a query that isn't a subquery, `Outer` is `Nil` and
    /// this is an uncorrelated `EXISTS`.
    pub fn exists<Idx>(&self) -> Exists<D, Outer::Tables>
    where
        Sel: Selection<Scope, Idx>,
    {
        self.exists_kind::<Idx>(false)
    }

    pub fn not_exists<Idx>(&self) -> Exists<D, Outer::Tables>
    where
        Sel: Selection<Scope, Idx>,
    {
        self.exists_kind::<Idx>(true)
    }

    fn exists_kind<Idx>(&self, negated: bool) -> Exists<D, Outer::Tables>
    where
        Sel: Selection<Scope, Idx>,
    {
        Exists {
            kind: ExprKind::Exists {
                body: Box::new(self.body.clone()),
                selection: self.selection.items(),
                negated,
            },
            _marker: PhantomData,
        }
    }

    /// `lhs IN (<this query>)`. This query selects exactly one column
    /// (`Sel: RowField` — a bare column, aggregate, or labelled one of
    /// those, never a tuple), so its SQL type can be checked against `lhs`
    /// the same way `.eq(..)` checks two columns: `RowField::Sql` carries
    /// the marker a decoded `Selection::Output` has already resolved away.
    /// Tagged with the outer tables `lhs` and this query's own `Outer`
    /// reference, so it can only be filtered onto a query that has both in
    /// scope.
    ///
    /// **Known limitation**: membership only. A *scalar* subquery
    /// (`col = (SELECT max(x) ..)`) stays deferred — it would have to be an
    /// `Expr`, which carries no dialect to pin the subquery's capability
    /// check to.
    pub fn contains<Lhs, Idx>(
        &self,
        lhs: Lhs,
    ) -> InSubquery<D, <Outer::Tables as Concat<Lhs::Req>>::Output>
    where
        Lhs: IntoExpr,
        Lhs::Sql: Comparable<Sel::Sql>,
        Sel: RowField<Scope, Idx> + Selection<Scope, Idx>,
        Outer::Tables: Concat<Lhs::Req>,
    {
        self.in_subquery_kind::<Lhs, Idx>(lhs, false)
    }

    pub fn not_contains<Lhs, Idx>(
        &self,
        lhs: Lhs,
    ) -> InSubquery<D, <Outer::Tables as Concat<Lhs::Req>>::Output>
    where
        Lhs: IntoExpr,
        Lhs::Sql: Comparable<Sel::Sql>,
        Sel: RowField<Scope, Idx> + Selection<Scope, Idx>,
        Outer::Tables: Concat<Lhs::Req>,
    {
        self.in_subquery_kind::<Lhs, Idx>(lhs, true)
    }

    fn in_subquery_kind<Lhs, Idx>(
        &self,
        lhs: Lhs,
        negated: bool,
    ) -> InSubquery<D, <Outer::Tables as Concat<Lhs::Req>>::Output>
    where
        Lhs: IntoExpr,
        Lhs::Sql: Comparable<Sel::Sql>,
        Sel: RowField<Scope, Idx> + Selection<Scope, Idx>,
        Outer::Tables: Concat<Lhs::Req>,
    {
        InSubquery {
            kind: ExprKind::InSubquery {
                lhs: Box::new(lhs.into_expr().kind),
                body: Box::new(self.body.clone()),
                selection: self.selection.items(),
                negated,
            },
            _marker: PhantomData,
        }
    }
}

/// A number of rows — what a `LIMIT` and an `OFFSET` each are: an integer,
/// or a `prepare!{}` placeholder for one, so a paginated endpoint can
/// prepare its query once and vary the page. A trait rather than
/// `Into<i64>` so a `usize` page size — the shape a paginated handler
/// already has — goes in without a cast. Every numeric impl lands in
/// `0..=i64::MAX`: a negative count is not a query any database will run,
/// and a `usize` past `i64::MAX` is not a page anyone is asking for.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a number of rows",
    label = "an integer, or a `prepare!{{}}` placeholder of type `Integer`/`BigInt`"
)]
pub trait IntoRowCount {
    fn into_row_count(self) -> RowCount;
}

/// What a `LIMIT`/`OFFSET` clause holds. Opaque: the `IntoRowCount` impls
/// are the only way to make one.
#[derive(Debug, Clone)]
pub struct RowCount(RowCountKind);

#[derive(Debug, Clone)]
enum RowCountKind {
    /// Written into the SQL text: a page size is not a value the plan
    /// should be reused across.
    Literal(i64),
    /// A bound parameter, which is what a `prepare!{}` placeholder is.
    Bound(ExprKind),
}

macro_rules! into_row_count {
    ($($signed:ty),+ ; $($unsigned:ty),+) => {
        $(impl IntoRowCount for $signed {
            fn into_row_count(self) -> RowCount {
                RowCount(RowCountKind::Literal((self as i64).max(0)))
            }
        })+
        $(impl IntoRowCount for $unsigned {
            fn into_row_count(self) -> RowCount {
                RowCount(RowCountKind::Literal(i64::try_from(self).unwrap_or(i64::MAX)))
            }
        })+
    };
}
into_row_count!(i8, i16, i32, i64, isize ; u8, u16, u32, u64, usize);

/// A `prepare!{}` placeholder, or any other scope-free `Expr` of an integer type:
/// bound rather than written, so one prepared query serves every page. Only
/// the two integer types — a page is a number.
impl IntoRowCount for Expr<Nil, crate::expr::Integer> {
    fn into_row_count(self) -> RowCount {
        RowCount(RowCountKind::Bound(self.kind))
    }
}

impl IntoRowCount for Expr<Nil, crate::expr::BigInt> {
    fn into_row_count(self) -> RowCount {
        RowCount(RowCountKind::Bound(self.kind))
    }
}

/// `LIMIT`/`OFFSET`, with the filler a dialect needs when there's an offset
/// and no limit — a bare `OFFSET` is Postgres-only.
pub(crate) fn render_limit_offset<D: Dialect>(
    sink: &mut dyn Sink,
    limit: Option<&RowCount>,
    offset: Option<&RowCount>,
) {
    match (limit, offset, D::OFFSET_WITHOUT_LIMIT) {
        (Some(l), _, _) => {
            sink.text(" LIMIT ");
            render_row_count::<D>(sink, l);
        }
        (None, Some(_), Some(filler)) => {
            sink.text(" LIMIT ");
            sink.text(filler);
        }
        _ => {}
    }
    if let Some(o) = offset {
        sink.text(" OFFSET ");
        render_row_count::<D>(sink, o);
    }
}

fn render_row_count<D: Dialect>(sink: &mut dyn Sink, count: &RowCount) {
    match &count.0 {
        RowCountKind::Literal(n) => sink.text(&n.to_string()),
        RowCountKind::Bound(kind) => render_expr::<D>(kind, sink),
    }
}

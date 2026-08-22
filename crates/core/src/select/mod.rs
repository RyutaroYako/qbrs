//! The `SELECT` builder: `select(..).from(..).join(..).filter(..)
//! .order_by(..).limit(..)`, in SQL keyword order, with tables passed as
//! arguments rather than by turbofish.

use std::marker::PhantomData;

use crate::cte::Cte;
use crate::dialect::{Dialect, SupportsFullOuterJoin, SupportsRightJoin};
use crate::expr::{Bool, Expr, ExprKind, IntoExpr, SqlType, Value};
use crate::render::{
    Fragment, FragmentSink, QuerySink, SelectItem, Sink, render_and_list, render_expr,
    render_expr_list, render_order_by, render_select_list,
};
use crate::scope::{
    BaseTable, Cons, MapNullable, MaybeNull, Nil, NotNull, ScopeTables, Superset, Table, TableSlot,
};

mod dyn_select;
mod prepared;
mod selection;
mod set_op;

pub use crate::expr::SortDir;
pub use dyn_select::{CannotFilterAfterErase, DynSelect};
pub use prepared::{Prepared, PreparedParams, UnresolvedPlaceholder};
pub use selection::{All, AllColumns, RowField, Selection, SelectionPart};
pub use set_op::{Ordinal, OrdinalKey, SetOp, nth};

/// One `name AS (body)` binding, carried in by the `Cte` a query was
/// entered through.
#[derive(Clone)]
struct CteDef {
    name: &'static str,
    column_names: &'static [&'static str],
    body: Fragment,
}

#[derive(Clone)]
enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
}

#[derive(Clone)]
struct JoinClause {
    kind: JoinKind,
    table: &'static str,
    on: ExprKind,
}

/// An order-by key: a column/expression tagged with a direction. Built via
/// `.asc()`/`.desc()` on any column or expression (see `OrderExt` below),
/// so `.order_by(users::created_at.desc())` reads as one argument, not two.
#[derive(Clone)]
pub struct OrderKey<Req> {
    kind: ExprKind,
    dir: SortDir,
    _marker: PhantomData<Req>,
}

impl<Req> OrderKey<Req> {
    pub(crate) fn into_parts(self) -> (ExprKind, SortDir) {
        (self.kind, self.dir)
    }
}

pub trait OrderExt<S: SqlType>: IntoExpr<S> + Sized {
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
impl<S: SqlType, T: IntoExpr<S>> OrderExt<S> for T {}

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
    fn binding(self) -> Option<Cte<D, Self::Table>>;
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
    fn binding(self) -> Option<Cte<D, T>> {
        None
    }
}

impl<D, Marker: crate::cte::CteShape> JoinSource<D> for Cte<D, Marker> {
    type Table = Marker;
    fn binding(self) -> Option<Cte<D, Marker>> {
        Some(self)
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
/// query the keys are eventually given to.
pub fn sort_key<Scope, Req, Idxs>(key: OrderKey<Req>) -> SortKey<Scope>
where
    Scope: Superset<Req, Idxs>,
{
    SortKey {
        kind: key.kind,
        dir: key.dir,
        _marker: PhantomData,
    }
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

/// Discharges a grouping key's scope requirement.
pub fn grouping<Scope, S: SqlType, Req, Idxs>(key: impl IntoExpr<S, Req = Req>) -> Grouping<Scope>
where
    Scope: Superset<Req, Idxs>,
{
    Grouping {
        kind: key.into_expr().kind,
        _marker: PhantomData,
    }
}

/// Something a query can be filtered by: an `Expr` whose tables this scope
/// contains, or a `Predicate` already discharged against it. One `.filter`
/// for both, so which one a condition happens to be doesn't change how it is
/// applied.
#[diagnostic::on_unimplemented(
    message = "`{Self}` isn't a condition",
    label = "a comparison (`.eq(..)`, `.gt(..)`, `.is_null()`), an `any_of`/`all_of` of them, a `sql!` fragment of type `Bool`, or a `predicate(..)`"
)]
pub trait Condition<Scope, Idxs> {
    /// Discharged against this scope, which for an `Expr` is where its
    /// `Superset` proof is spent and for a `Predicate` already happened.
    fn into_predicate(self) -> Predicate<Scope>;
}

#[diagnostic::do_not_recommend]
impl<Scope: Superset<Req, Idxs>, Req, Idxs> Condition<Scope, Idxs> for Expr<Req, Bool> {
    fn into_predicate(self) -> Predicate<Scope> {
        Predicate {
            kind: self.kind,
            _marker: PhantomData,
        }
    }
}

impl<Scope> Condition<Scope, ()> for Predicate<Scope> {
    fn into_predicate(self) -> Predicate<Scope> {
        self
    }
}

/// A condition whose scope requirement has already been discharged, so a
/// runtime-length collection of them can be built and passed around. An
/// `Expr` carries the tables it references in its type, which is what makes
/// `vec![users_cond, orders_cond]` fail to unify; `predicate` trades that
/// tag for a proof against one concrete scope.
pub struct Predicate<Scope> {
    kind: ExprKind,
    _marker: PhantomData<fn() -> Scope>,
}

// Hand-written for the reason `Expr`'s is: `#[derive(Clone)]` would ask a
// phantom `Scope` to be `Clone`.
impl<Scope> Clone for Predicate<Scope> {
    fn clone(&self) -> Self {
        Predicate {
            kind: self.kind.clone(),
            _marker: PhantomData,
        }
    }
}

impl<Scope> Predicate<Scope> {
    /// True when any of them is. `expr::any_of` combines conditions that
    /// reference the same tables; this one combines conditions whose scope
    /// requirement is already discharged, which is what lets a search form
    /// OR together conditions from different tables.
    pub fn any(preds: impl IntoIterator<Item = Predicate<Scope>>) -> Self {
        Predicate::combine(preds, false)
    }

    /// True when all of them are — the AND to `any`'s OR, so a group of
    /// them can be nested inside one.
    pub fn all(preds: impl IntoIterator<Item = Predicate<Scope>>) -> Self {
        Predicate::combine(preds, true)
    }

    fn combine(preds: impl IntoIterator<Item = Predicate<Scope>>, all: bool) -> Self {
        Predicate {
            kind: crate::expr::fold_conditions(preds.into_iter().map(Predicate::into_kind), all),
            _marker: PhantomData,
        }
    }

    pub(crate) fn into_kind(self) -> ExprKind {
        self.kind
    }
}

/// Discharges a condition's scope requirement. `Scope` is inferred from the
/// query the resulting predicates are eventually given to.
pub fn predicate<Scope, Req, Idxs>(cond: Expr<Req, Bool>) -> Predicate<Scope>
where
    Scope: Superset<Req, Idxs>,
{
    Predicate {
        kind: cond.kind,
        _marker: PhantomData,
    }
}

/// Holds just the `SELECT` list until `.from(..)` supplies the first table
/// and therefore the query's initial `Scope`. Splitting this out (rather
/// than requiring `Scope` be known at `.select(..)` time) is what lets the
/// builder read in `SELECT -> FROM -> ...` order while still validating
/// every selected column against the *final* scope only once, at the
/// query's terminal method (`.to_sql()`/`.load()`).
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
#[derive(Clone)]
pub(super) struct SelectBody {
    ctes: Vec<CteDef>,
    distinct: bool,
    from_table: &'static str,
    joins: Vec<JoinClause>,
    wheres: Vec<ExprKind>,
    order_by: Vec<(ExprKind, SortDir)>,
    group_by: Vec<ExprKind>,
    having: Vec<ExprKind>,
    limit: Option<Limit>,
    offset: Option<Limit>,
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
    /// A query whose rows aren't one per matching row — `GROUP BY`,
    /// `HAVING`, `DISTINCT` — is counted by wrapping it, selection and all,
    /// since what a page of it would show is what has to be counted.
    fn count_sql<D: Dialect>(&self, selection: &[SelectItem]) -> (String, Vec<Value>) {
        let mut body = self.clone();
        body.order_by.clear();
        body.limit = None;
        body.offset = None;
        let one_row_each = body.group_by.is_empty() && body.having.is_empty() && !body.distinct;

        let mut sink = QuerySink::<D>::new();
        if one_row_each {
            body.render_into::<D>(&[crate::expr::count_item()], &mut sink);
            return sink.finish();
        }
        sink.text("SELECT count(*) FROM (");
        body.render_into::<D>(selection, &mut sink);
        sink.text(") AS ");
        crate::render::render_ident::<D>(&mut sink, "qbrs_total");
        sink.finish()
    }

    /// Attaches whatever `WITH` binding a join source carries before its
    /// table is named in a FROM or JOIN clause.
    fn bind<D, S: JoinSource<D>>(&mut self, source: S) {
        if let Some(cte) = source.binding() {
            self.ctes.push(CteDef {
                name: cte.name,
                column_names: cte.column_names,
                body: cte.body,
            });
        }
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
    pub fn filter<C: Condition<Scope, Idxs>, Idxs>(mut self, cond: C) -> Self {
        self.body.wheres.push(cond.into_predicate().into_kind());
        self
    }

    /// AND-folds a runtime-length collection of already-discharged
    /// conditions — the shape a search form has, where the conditions come
    /// from different tables and so can't share one `Expr` type.
    pub fn filter_all(mut self, conds: impl IntoIterator<Item = Predicate<Scope>>) -> Self {
        self.body
            .wheres
            .extend(conds.into_iter().map(Predicate::into_kind));
        self
    }

    pub fn order_by<Req, Idxs>(mut self, key: OrderKey<Req>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
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
    pub fn distinct(mut self) -> Self {
        self.body.distinct = true;
        self
    }

    /// Appends one grouping key; callable multiple times like `.filter()`
    /// (each call adds a column to the `GROUP BY` list, it doesn't replace
    /// it), for the same "dynamic composition without a type change" reason.
    pub fn group_by<S: SqlType, Req, Idxs>(mut self, key: impl IntoExpr<S, Req = Req>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.body.group_by.push(key.into_expr().kind);
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
    pub fn having<C: Condition<Scope, Idxs>, Idxs>(mut self, cond: C) -> Self {
        self.body.having.push(cond.into_predicate().into_kind());
        self
    }

    pub fn limit(mut self, n: impl IntoLimit) -> Self {
        self.body.limit = Some(n.into_limit());
        self
    }

    pub fn offset(mut self, n: impl IntoLimit) -> Self {
        self.body.offset = Some(n.into_limit());
        self
    }

    /// `New` and `Req` are both inferred from the arguments (the table
    /// token's type, and the `on` expression's own tracked requirement) —
    /// no turbofish, no closure.
    pub fn inner_join<S: JoinSource<D>, Req, Idxs>(
        mut self,
        source: S,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<S::Table, NotNull>, Scope>, Sel, Outer>
    where
        Cons<TableSlot<S::Table, NotNull>, Scope>: Superset<Req, Idxs>,
    {
        self.body.bind(source);
        self.body.joins.push(JoinClause {
            kind: JoinKind::Inner,
            table: <S::Table as Table>::NAME,
            on: on.kind,
        });
        self.retype()
    }

    pub fn left_join<S: JoinSource<D>, Req, Idxs>(
        mut self,
        source: S,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<S::Table, MaybeNull>, Scope>, Sel, Outer>
    where
        Cons<TableSlot<S::Table, MaybeNull>, Scope>: Superset<Req, Idxs>,
    {
        self.body.bind(source);
        self.body.joins.push(JoinClause {
            kind: JoinKind::Left,
            table: <S::Table as Table>::NAME,
            on: on.kind,
        });
        self.retype()
    }

    /// RIGHT JOIN retroactively flips every already-joined table to
    /// nullable (`MapNullable`) before adding the new, guaranteed-present
    /// table, mirroring Drizzle's `AppendToNullabilityMap` rule.
    pub fn right_join<S: JoinSource<D>, Req, Idxs>(
        mut self,
        source: S,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<S::Table, NotNull>, Scope::Output>, Sel, Outer>
    where
        D: SupportsRightJoin,
        Scope: MapNullable,
        Cons<TableSlot<S::Table, NotNull>, Scope::Output>: Superset<Req, Idxs>,
    {
        self.body.bind(source);
        self.body.joins.push(JoinClause {
            kind: JoinKind::Right,
            table: <S::Table as Table>::NAME,
            on: on.kind,
        });
        self.retype()
    }

    pub fn full_join<S: JoinSource<D>, Req, Idxs>(
        mut self,
        source: S,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<S::Table, MaybeNull>, Scope::Output>, Sel, Outer>
    where
        D: SupportsFullOuterJoin,
        Scope: MapNullable,
        Cons<TableSlot<S::Table, MaybeNull>, Scope::Output>: Superset<Req, Idxs>,
    {
        self.body.bind(source);
        self.body.joins.push(JoinClause {
            kind: JoinKind::Full,
            table: <S::Table as Table>::NAME,
            on: on.kind,
        });
        self.retype()
    }
}

impl<D: Dialect, Scope, Sel, Outer> Select<D, Scope, Sel, Outer> {
    /// The terminal step, and the *only* point each selected column's
    /// scope-membership is checked — proven as a side effect of
    /// `Sel: Selection<Scope, Idx>` type-checking at all.
    pub fn to_sql<Idx>(&self) -> (String, Vec<Value>)
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
    pub fn count_sql<Idx>(&self) -> (String, Vec<Value>)
    where
        Sel: Selection<Scope, Idx>,
    {
        self.body.count_sql::<D>(&self.selection.items())
    }

    /// This query as an embeddable `Fragment`: an `EXISTS (..)` subquery, a
    /// CTE body, or a set-operation branch. The only way to produce one, so
    /// no caller has to remember that an embedded query renders with `?`
    /// placeholders rather than `D`'s own style.
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
    pub(super) fn render_into<D: Dialect>(&self, selection: &[SelectItem], sink: &mut dyn Sink) {
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
        let mut body = SelectBody::new(<S::Table as Table>::NAME);
        body.bind(source);
        Select {
            body,
            selection,
            _marker: PhantomData,
        }
    }
}

impl<D: Dialect, Scope, Sel, Outer: ScopeTables> Select<D, Scope, Sel, Outer> {
    /// `EXISTS (<this query>)`, tagged with the outer tables it references
    /// so it can only be filtered onto a query that has them in scope. Its
    /// own column references were already checked against `Scope` when it
    /// was built. On a query that isn't a subquery, `Outer` is `Nil` and
    /// this is an uncorrelated `EXISTS`.
    pub fn exists<Idx>(&self) -> Expr<Outer::Tables, Bool>
    where
        Sel: Selection<Scope, Idx>,
    {
        Expr::from_kind(ExprKind::Raw(
            self.fragment::<Idx>().enclosed_in("EXISTS (", ")"),
        ))
    }

    pub fn not_exists<Idx>(&self) -> Expr<Outer::Tables, Bool>
    where
        Sel: Selection<Scope, Idx>,
    {
        Expr::from_kind(ExprKind::Raw(
            self.fragment::<Idx>().enclosed_in("NOT EXISTS (", ")"),
        ))
    }
}

/// A limit or offset: a number, or a `prepare!{}` placeholder for one, so a
/// paginated endpoint can prepare its query once and vary the page. A trait
/// rather than `Into<i64>` so a `usize` page size — the shape a paginated
/// handler already has — goes in without a cast. Every numeric impl lands in
/// `0..=i64::MAX`: a negative limit is not a query any database will run,
/// and a `usize` past `i64::MAX` is not a page anyone is asking for.
pub trait IntoLimit {
    fn into_limit(self) -> Limit;
}

/// What a `LIMIT`/`OFFSET` clause holds. Opaque: the `IntoLimit` impls are
/// the only way to make one.
#[derive(Clone)]
pub struct Limit(LimitKind);

#[derive(Clone)]
enum LimitKind {
    /// Written into the SQL text: a page size is not a value the plan
    /// should be reused across.
    Literal(i64),
    /// A bound parameter, which is what a `prepare!{}` placeholder is.
    Bound(ExprKind),
}

macro_rules! into_limit {
    ($($signed:ty),+ ; $($unsigned:ty),+) => {
        $(impl IntoLimit for $signed {
            fn into_limit(self) -> Limit {
                Limit(LimitKind::Literal(i64::from(self).max(0)))
            }
        })+
        $(impl IntoLimit for $unsigned {
            fn into_limit(self) -> Limit {
                Limit(LimitKind::Literal(i64::try_from(self).unwrap_or(i64::MAX)))
            }
        })+
    };
}
into_limit!(i32 ; u8, u16, u32, u64, usize);

impl IntoLimit for i64 {
    fn into_limit(self) -> Limit {
        Limit(LimitKind::Literal(self.max(0)))
    }
}

/// A `prepare!{}` placeholder, or any other scope-free integer expression:
/// bound rather than written, so one prepared query serves every page.
impl<S: SqlType> IntoLimit for Expr<Nil, S> {
    fn into_limit(self) -> Limit {
        Limit(LimitKind::Bound(self.kind))
    }
}

/// `LIMIT`/`OFFSET`, with the filler a dialect needs when there's an offset
/// and no limit — a bare `OFFSET` is Postgres-only.
pub(crate) fn render_limit_offset<D: Dialect>(
    sink: &mut dyn Sink,
    limit: Option<&Limit>,
    offset: Option<&Limit>,
) {
    match (limit, offset, D::OFFSET_WITHOUT_LIMIT) {
        (Some(l), _, _) => {
            sink.text(" LIMIT ");
            render_limit::<D>(sink, l);
        }
        (None, Some(_), Some(filler)) => {
            sink.text(" LIMIT ");
            sink.text(filler);
        }
        _ => {}
    }
    if let Some(o) = offset {
        sink.text(" OFFSET ");
        render_limit::<D>(sink, o);
    }
}

fn render_limit<D: Dialect>(sink: &mut dyn Sink, limit: &Limit) {
    match &limit.0 {
        LimitKind::Literal(n) => sink.text(&n.to_string()),
        LimitKind::Bound(kind) => render_expr::<D>(kind, sink),
    }
}

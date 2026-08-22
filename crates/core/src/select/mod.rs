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
pub use dyn_select::DynSelect;
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

/// A condition whose scope requirement has already been discharged, so a
/// runtime-length collection of them can be built and passed around. An
/// `Expr` carries the tables it references in its type, which is what makes
/// `vec![users_cond, orders_cond]` fail to unify; `predicate` trades that
/// tag for a proof against one concrete scope.
pub struct Predicate<Scope> {
    kind: ExprKind,
    _marker: PhantomData<fn() -> Scope>,
}

impl<Scope> Predicate<Scope> {
    /// True when any of them is. `expr::any_of` combines conditions that
    /// reference the same tables; this one combines conditions whose scope
    /// requirement is already discharged, which is what lets a search form
    /// OR together conditions from different tables.
    pub fn any(preds: impl IntoIterator<Item = Predicate<Scope>>) -> Self {
        Predicate::combine(preds, false)
    }

    /// True when all of them are.
    pub fn all(preds: impl IntoIterator<Item = Predicate<Scope>>) -> Self {
        Predicate::combine(preds, true)
    }

    fn combine(preds: impl IntoIterator<Item = Predicate<Scope>>, all: bool) -> Self {
        let kinds = preds.into_iter().map(Predicate::into_kind);
        let mut folded: Option<ExprKind> = None;
        for kind in kinds {
            folded = Some(match folded {
                None => kind,
                Some(acc) if all => ExprKind::And(Box::new(acc), Box::new(kind)),
                Some(acc) => ExprKind::Or(Box::new(acc), Box::new(kind)),
            });
        }
        Predicate {
            kind: folded.unwrap_or(ExprKind::Always(all)),
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
    limit: Option<i64>,
    offset: Option<i64>,
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

pub struct Select<D, Scope, Sel> {
    body: SelectBody,
    selection: Sel,
    _marker: PhantomData<fn() -> (D, Scope)>,
}

// Cloning is what lets one built-up query serve both a count and a page.
impl<D, Scope, Sel: Clone> Clone for Select<D, Scope, Sel> {
    fn clone(&self) -> Self {
        Select {
            body: self.body.clone(),
            selection: self.selection.clone(),
            _marker: PhantomData,
        }
    }
}

impl<D, Scope, Sel> Select<D, Scope, Sel> {
    fn retype<NewScope>(self) -> Select<D, NewScope, Sel> {
        Select {
            body: self.body,
            selection: self.selection,
            _marker: PhantomData,
        }
    }

    /// Swaps the selection list, keeping every clause. With `Clone`, this is
    /// how one built-up query serves both a count and a page.
    pub fn reselect<NewSel>(self, selection: NewSel) -> Select<D, Scope, NewSel> {
        Select {
            body: self.body,
            selection,
            _marker: PhantomData,
        }
    }

    /// AND-folded, and callable any number of times — conditionally, in a
    /// loop, from a helper — without changing `Self`'s type, so the most
    /// common kind of dynamic query needs no escape hatch.
    pub fn filter<Req, Idxs>(mut self, cond: Expr<Req, Bool>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.body.wheres.push(cond.kind);
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

    /// A `WHERE`-shaped filter applied after grouping (aggregate
    /// conditions) — AND-folded across calls exactly like `.filter()`.
    pub fn having<Req, Idxs>(mut self, cond: Expr<Req, Bool>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.body.having.push(cond.kind);
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
    ) -> Select<D, Cons<TableSlot<S::Table, NotNull>, Scope>, Sel>
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
    ) -> Select<D, Cons<TableSlot<S::Table, MaybeNull>, Scope>, Sel>
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
    ) -> Select<D, Cons<TableSlot<S::Table, NotNull>, Scope::Output>, Sel>
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
    ) -> Select<D, Cons<TableSlot<S::Table, MaybeNull>, Scope::Output>, Sel>
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

impl<D: Dialect, Scope, Sel> Select<D, Scope, Sel> {
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
    pub(super) fn render_into<RD: Dialect>(&self, selection: &[SelectItem], sink: &mut dyn Sink) {
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
        let (limit, offset) = (*limit, *offset);
        if !ctes.is_empty() {
            sink.text("WITH ");
            for (i, cte) in ctes.iter().enumerate() {
                if i > 0 {
                    sink.text(", ");
                }
                crate::render::render_ident::<RD>(sink, cte.name);
                sink.text(" (");
                for (i, col) in cte.column_names.iter().enumerate() {
                    if i > 0 {
                        sink.text(", ");
                    }
                    crate::render::render_ident::<RD>(sink, col);
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

        render_select_list::<RD>(selection, sink);

        sink.text(" FROM ");
        crate::render::render_ident::<RD>(sink, from_table);

        for j in joins {
            sink.ch(' ');
            sink.text(match j.kind {
                JoinKind::Inner => "INNER JOIN",
                JoinKind::Left => "LEFT JOIN",
                JoinKind::Right => "RIGHT JOIN",
                JoinKind::Full => "FULL JOIN",
            });
            sink.ch(' ');
            crate::render::render_ident::<RD>(sink, j.table);
            sink.text(" ON ");
            render_expr::<RD>(&j.on, sink);
        }

        render_and_list::<RD>(sink, " WHERE ", wheres);

        render_expr_list::<RD>(sink, " GROUP BY ", group_by);
        render_and_list::<RD>(sink, " HAVING ", having);
        render_order_by::<RD>(sink, " ORDER BY ", order_by);
        render_limit_offset::<RD>(sink, limit, offset);
    }
}

/// A subquery built against an outer query's scope. Distinct from `Select`
/// so `EXISTS` can report the outer tables it references: those are exactly
/// `Outer`'s, and an `EXISTS` condition is only usable where they are all in
/// scope.
pub struct Correlated<D, Outer, Scope, Sel> {
    inner: Select<D, Scope, Sel>,
    _marker: PhantomData<fn() -> Outer>,
}

impl<D, Scope, Sel> Select<D, Scope, Sel> {
    /// Starts a correlated subquery: a fresh `SELECT` whose scope is
    /// `Cons<TableSlot<T, NotNull>, Scope>` — the new table, prepended onto
    /// *this* (outer) query's entire scope. Because `Find`/`Superset` walk
    /// the whole flat cons-list regardless of where it came from, the
    /// subquery's `.filter()` can reference both its own new table's
    /// columns and any outer column already in `Scope`, with no special
    /// casing: growing the scope works the same whether the new table came
    /// from a join or from a subquery's `FROM`.
    pub fn correlated<S: JoinSource<D>, InnerSel>(
        &self,
        source: S,
        selection: InnerSel,
    ) -> Correlated<D, Scope, Cons<TableSlot<S::Table, NotNull>, Scope>, InnerSel> {
        let mut body = SelectBody::new(<S::Table as Table>::NAME);
        body.bind(source);
        Correlated {
            inner: Select {
                body,
                selection,
                _marker: PhantomData,
            },
            _marker: PhantomData,
        }
    }
}

impl<D, Outer, Scope, Sel> Correlated<D, Outer, Scope, Sel> {
    /// A subquery is a `Select` with one extra promise — that its scope
    /// starts where the outer query's left off — so every clause it takes is
    /// the `Select` method applied to the query inside, and every join is
    /// that plus a new `Scope`.
    fn map<NewScope>(
        self,
        f: impl FnOnce(Select<D, Scope, Sel>) -> Select<D, NewScope, Sel>,
    ) -> Correlated<D, Outer, NewScope, Sel> {
        Correlated {
            inner: f(self.inner),
            _marker: PhantomData,
        }
    }

    /// Checked against the subquery's own scope, which includes every table
    /// the outer query had — that is what makes it *correlated*.
    pub fn filter<Req, Idxs>(self, cond: Expr<Req, Bool>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.map(|q| q.filter(cond))
    }

    /// AND-folds a runtime-length collection of discharged conditions, the
    /// same way `Select::filter_all` does.
    pub fn filter_all(self, conds: impl IntoIterator<Item = Predicate<Scope>>) -> Self {
        self.map(|q| q.filter_all(conds))
    }

    pub fn group_by<S: SqlType, Req, Idxs>(self, key: impl IntoExpr<S, Req = Req>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.map(|q| q.group_by(key))
    }

    pub fn having<Req, Idxs>(self, cond: Expr<Req, Bool>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.map(|q| q.having(cond))
    }

    pub fn order_by<Req, Idxs>(self, key: OrderKey<Req>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.map(|q| q.order_by(key))
    }

    pub fn limit(self, n: impl IntoLimit) -> Self {
        self.map(|q| q.limit(n))
    }

    pub fn offset(self, n: impl IntoLimit) -> Self {
        self.map(|q| q.offset(n))
    }

    pub fn inner_join<S: JoinSource<D>, Req, Idxs>(
        self,
        source: S,
        on: Expr<Req, Bool>,
    ) -> Correlated<D, Outer, Cons<TableSlot<S::Table, NotNull>, Scope>, Sel>
    where
        Cons<TableSlot<S::Table, NotNull>, Scope>: Superset<Req, Idxs>,
    {
        self.map(|q| q.inner_join(source, on))
    }

    pub fn left_join<S: JoinSource<D>, Req, Idxs>(
        self,
        source: S,
        on: Expr<Req, Bool>,
    ) -> Correlated<D, Outer, Cons<TableSlot<S::Table, MaybeNull>, Scope>, Sel>
    where
        Cons<TableSlot<S::Table, MaybeNull>, Scope>: Superset<Req, Idxs>,
    {
        self.map(|q| q.left_join(source, on))
    }

    pub fn right_join<S: JoinSource<D>, Req, Idxs>(
        self,
        source: S,
        on: Expr<Req, Bool>,
    ) -> Correlated<D, Outer, Cons<TableSlot<S::Table, NotNull>, Scope::Output>, Sel>
    where
        D: SupportsRightJoin,
        Scope: MapNullable,
        Cons<TableSlot<S::Table, NotNull>, Scope::Output>: Superset<Req, Idxs>,
    {
        self.map(|q| q.right_join(source, on))
    }

    pub fn full_join<S: JoinSource<D>, Req, Idxs>(
        self,
        source: S,
        on: Expr<Req, Bool>,
    ) -> Correlated<D, Outer, Cons<TableSlot<S::Table, MaybeNull>, Scope::Output>, Sel>
    where
        D: SupportsFullOuterJoin,
        Scope: MapNullable,
        Cons<TableSlot<S::Table, MaybeNull>, Scope::Output>: Superset<Req, Idxs>,
    {
        self.map(|q| q.full_join(source, on))
    }
}

impl<D: Dialect, Outer: ScopeTables, Scope, Sel> Correlated<D, Outer, Scope, Sel> {
    /// `EXISTS (<this subquery>)`, tagged with the outer tables it
    /// references so it can only be filtered onto a query that has them in
    /// scope. Its own column references were already checked against
    /// `Scope` when it was built.
    pub fn exists<Idx>(&self) -> Expr<Outer::Tables, Bool>
    where
        Sel: Selection<Scope, Idx>,
    {
        Expr::from_kind(ExprKind::Raw(
            self.inner.fragment::<Idx>().enclosed_in("EXISTS (", ")"),
        ))
    }

    pub fn not_exists<Idx>(&self) -> Expr<Outer::Tables, Bool>
    where
        Sel: Selection<Scope, Idx>,
    {
        Expr::from_kind(ExprKind::Raw(
            self.inner
                .fragment::<Idx>()
                .enclosed_in("NOT EXISTS (", ")"),
        ))
    }
}

/// A limit or offset. A trait rather than `Into<i64>` so a `usize` page size
/// — the shape a paginated handler already has — goes in without a cast.
pub trait IntoLimit {
    fn into_limit(self) -> i64;
}

macro_rules! into_limit {
    ($($ty:ty),+) => { $( impl IntoLimit for $ty { fn into_limit(self) -> i64 { self as i64 } } )+ };
}
into_limit!(i32, i64, u8, u16, u32, usize);

/// `LIMIT`/`OFFSET`, with the filler a dialect needs when there's an offset
/// and no limit — a bare `OFFSET` is Postgres-only.
pub(crate) fn render_limit_offset<D: Dialect>(
    sink: &mut dyn Sink,
    limit: Option<i64>,
    offset: Option<i64>,
) {
    match (limit, offset, D::OFFSET_WITHOUT_LIMIT) {
        (Some(l), _, _) => {
            sink.text(" LIMIT ");
            sink.text(&l.to_string());
        }
        (None, Some(_), Some(filler)) => {
            sink.text(" LIMIT ");
            sink.text(filler);
        }
        _ => {}
    }
    if let Some(o) = offset {
        sink.text(" OFFSET ");
        sink.text(&o.to_string());
    }
}

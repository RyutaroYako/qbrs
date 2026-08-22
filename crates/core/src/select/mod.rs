//! The `SELECT` builder: `select(..).from(..).join(..).filter(..)
//! .order_by(..).limit(..)`, in SQL keyword order, with tables passed as
//! arguments rather than by turbofish.

use std::marker::PhantomData;

use crate::dialect::{Dialect, RawEmbed, SupportsFullOuterJoin, SupportsRightJoin};
use crate::expr::{Bool, Expr, ExprKind, IntoExpr, SqlType, Value};
use crate::render::{Fragment, SelectItem, render_expr, render_select_list};
use crate::scope::{
    BaseTable, Cons, MapNullable, MaybeNull, Nil, NotNull, ScopeTables, Superset, TableSlot,
};

mod dyn_select;
mod prepared;
mod selection;
mod set_op;

pub use crate::expr::SortDir;
pub use dyn_select::DynSelect;
pub use prepared::{Prepared, PreparedParams, UnresolvedPlaceholder};
pub use selection::{RowField, Selection};
pub use set_op::SetOp;

/// One `name AS (body)` binding accumulated by `SelectSeed::with`.
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

/// A condition whose scope requirement has already been discharged, so a
/// runtime-length collection of them can be built and passed around. An
/// `Expr` carries the tables it references in its type, which is what makes
/// `vec![users_cond, orders_cond]` fail to unify; `predicate` trades that
/// tag for a proof against one concrete scope.
pub struct Predicate<Scope> {
    kind: ExprKind,
    _marker: PhantomData<fn() -> Scope>,
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
    /// `table` is a value (the zero-sized token the schema macro generates,
    /// e.g. `users::Table`), not a turbofish — `D` and `T` are inferred
    /// from how the resulting `Select` is eventually used (its dialect from
    /// `.load(&db)`, its table from the argument's own type).
    pub fn from<D, T: BaseTable>(
        self,
        _table: T,
    ) -> Select<D, Cons<TableSlot<T, NotNull>, Nil>, Sel> {
        Select {
            body: SelectBody::new(T::NAME, Vec::new()),
            selection: self.selection,
            _marker: PhantomData,
        }
    }

    /// `FROM` a common table expression. Binding it and putting it in scope
    /// are the same act, so a `with!{}` pseudo-table cannot be selected from
    /// unless a `cte::with(..)` for it was passed here — and the query takes
    /// the binding's dialect, so a body rendered for one dialect cannot be
    /// spliced into another's statement.
    ///
    /// Callable once; `.join_cte(..)` attaches any further ones. **Known
    /// limitation**: a CTE body can't reference another CTE, so the order
    /// they're attached in doesn't matter.
    pub fn from_cte<D, Marker: crate::cte::CteShape>(
        self,
        cte: crate::cte::Cte<D, Marker>,
    ) -> Select<D, Cons<TableSlot<Marker, NotNull>, Nil>, Sel> {
        let mut body = SelectBody::new(Marker::NAME, Vec::new());
        body.ctes.push(CteDef {
            name: cte.name,
            column_names: cte.column_names,
            body: cte.body,
        });
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
    fn new(from_table: &'static str, ctes: Vec<CteDef>) -> Self {
        SelectBody {
            ctes,
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
        self.body.wheres.extend(conds.into_iter().map(|p| p.kind));
        self
    }

    pub fn order_by<Req, Idxs>(mut self, key: OrderKey<Req>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.body.order_by.push((key.kind, key.dir));
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

    pub fn limit(mut self, n: impl crate::row::IntoLimit) -> Self {
        self.body.limit = Some(n.into_limit());
        self
    }

    pub fn offset(mut self, n: impl crate::row::IntoLimit) -> Self {
        self.body.offset = Some(n.into_limit());
        self
    }

    /// `New` and `Req` are both inferred from the arguments (the table
    /// token's type, and the `on` expression's own tracked requirement) —
    /// no turbofish, no closure.
    pub fn inner_join<New: BaseTable, Req, Idxs>(
        mut self,
        _table: New,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<New, NotNull>, Scope>, Sel>
    where
        Cons<TableSlot<New, NotNull>, Scope>: Superset<Req, Idxs>,
    {
        self.body.joins.push(JoinClause {
            kind: JoinKind::Inner,
            table: New::NAME,
            on: on.kind,
        });
        self.retype()
    }

    pub fn left_join<New: BaseTable, Req, Idxs>(
        mut self,
        _table: New,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<New, MaybeNull>, Scope>, Sel>
    where
        Cons<TableSlot<New, MaybeNull>, Scope>: Superset<Req, Idxs>,
    {
        self.body.joins.push(JoinClause {
            kind: JoinKind::Left,
            table: New::NAME,
            on: on.kind,
        });
        self.retype()
    }

    /// `INNER JOIN` a common table expression, attaching its `WITH` binding
    /// at the same time — see `SelectSeed::from_cte` for why the two are one
    /// act.
    pub fn inner_join_cte<Marker: crate::cte::CteShape, Req, Idxs>(
        mut self,
        cte: crate::cte::Cte<D, Marker>,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<Marker, NotNull>, Scope>, Sel>
    where
        Cons<TableSlot<Marker, NotNull>, Scope>: Superset<Req, Idxs>,
    {
        self.body.ctes.push(CteDef {
            name: cte.name,
            column_names: cte.column_names,
            body: cte.body,
        });
        self.body.joins.push(JoinClause {
            kind: JoinKind::Inner,
            table: Marker::NAME,
            on: on.kind,
        });
        self.retype()
    }

    pub fn left_join_cte<Marker: crate::cte::CteShape, Req, Idxs>(
        mut self,
        cte: crate::cte::Cte<D, Marker>,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<Marker, MaybeNull>, Scope>, Sel>
    where
        Cons<TableSlot<Marker, MaybeNull>, Scope>: Superset<Req, Idxs>,
    {
        self.body.ctes.push(CteDef {
            name: cte.name,
            column_names: cte.column_names,
            body: cte.body,
        });
        self.body.joins.push(JoinClause {
            kind: JoinKind::Left,
            table: Marker::NAME,
            on: on.kind,
        });
        self.retype()
    }

    /// RIGHT JOIN retroactively flips every already-joined table to
    /// nullable (`MapNullable`) before adding the new, guaranteed-present
    /// table, mirroring Drizzle's `AppendToNullabilityMap` rule.
    pub fn right_join<New: BaseTable, Req, Idxs>(
        mut self,
        _table: New,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<New, NotNull>, Scope::Output>, Sel>
    where
        D: SupportsRightJoin,
        Scope: MapNullable,
        Cons<TableSlot<New, NotNull>, Scope::Output>: Superset<Req, Idxs>,
    {
        self.body.joins.push(JoinClause {
            kind: JoinKind::Right,
            table: New::NAME,
            on: on.kind,
        });
        self.retype()
    }

    pub fn full_join<New: BaseTable, Req, Idxs>(
        mut self,
        _table: New,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<New, MaybeNull>, Scope::Output>, Sel>
    where
        D: SupportsFullOuterJoin,
        Scope: MapNullable,
        Cons<TableSlot<New, MaybeNull>, Scope::Output>: Superset<Req, Idxs>,
    {
        self.body.joins.push(JoinClause {
            kind: JoinKind::Full,
            table: New::NAME,
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
        self.render_as::<D, Idx>()
    }

    /// `SELECT count(*)` over this query's `FROM`/`JOIN`/`WHERE`/`GROUP BY`,
    /// dropping its `ORDER BY`/`LIMIT`/`OFFSET` — a total is about the rows
    /// that match, not the page being shown. `reselect(count())` keeps them,
    /// which is what makes it the wrong tool for a paginated total.
    pub fn count_sql(&self) -> (String, Vec<Value>) {
        let mut body = self.body.clone();
        body.order_by.clear();
        body.limit = None;
        body.offset = None;
        body.render::<D>(&[crate::expr::count_item()])
    }

    /// This query as an embeddable `Fragment`: an `EXISTS (..)` subquery, a
    /// CTE body, or a set-operation branch. The only way to produce one, so
    /// no caller has to remember that an embedded query renders with `?`
    /// placeholders rather than `D`'s own style.
    pub(crate) fn fragment<Idx>(&self) -> Fragment
    where
        Sel: Selection<Scope, Idx>,
    {
        let (sql, params) = self.render_as::<RawEmbed<D>, Idx>();
        Fragment::from_rendered(&sql, params)
    }

    fn render_as<RD: Dialect, Idx>(&self) -> (String, Vec<Value>)
    where
        Sel: Selection<Scope, Idx>,
    {
        self.body.render::<RD>(&self.selection.items())
    }
}

impl SelectBody {
    pub(super) fn render<RD: Dialect>(&self, selection: &[SelectItem]) -> (String, Vec<Value>) {
        let SelectBody {
            ctes,
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
        let mut sql = String::new();
        let mut params = Vec::new();

        if !ctes.is_empty() {
            sql.push_str("WITH ");
            for (i, cte) in ctes.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                crate::render::render_ident::<RD>(&mut sql, cte.name);
                sql.push_str(" (");
                for (i, col) in cte.column_names.iter().enumerate() {
                    if i > 0 {
                        sql.push_str(", ");
                    }
                    crate::render::render_ident::<RD>(&mut sql, col);
                }
                sql.push_str(") AS (");
                cte.body.splice_into::<RD>(&mut sql, &mut params);
                sql.push(')');
            }
            sql.push(' ');
        }
        sql.push_str("SELECT ");

        render_select_list::<RD>(selection, &mut sql, &mut params);

        sql.push_str(" FROM ");
        crate::render::render_ident::<RD>(&mut sql, from_table);

        for j in joins {
            sql.push(' ');
            sql.push_str(match j.kind {
                JoinKind::Inner => "INNER JOIN",
                JoinKind::Left => "LEFT JOIN",
                JoinKind::Right => "RIGHT JOIN",
                JoinKind::Full => "FULL JOIN",
            });
            sql.push(' ');
            crate::render::render_ident::<RD>(&mut sql, j.table);
            sql.push_str(" ON ");
            render_expr::<RD>(&j.on, &mut sql, &mut params);
        }

        push_and_list::<RD>(&mut sql, &mut params, " WHERE ", wheres);

        if !group_by.is_empty() {
            sql.push_str(" GROUP BY ");
            for (i, g) in group_by.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                render_expr::<RD>(g, &mut sql, &mut params);
            }
        }

        push_and_list::<RD>(&mut sql, &mut params, " HAVING ", having);

        if !order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            for (i, (e, dir)) in order_by.iter().enumerate() {
                if i > 0 {
                    sql.push_str(", ");
                }
                render_expr::<RD>(e, &mut sql, &mut params);
                sql.push_str(match dir {
                    SortDir::Asc => " ASC",
                    SortDir::Desc => " DESC",
                });
            }
        }

        if let Some(l) = limit {
            sql.push_str(" LIMIT ");
            sql.push_str(&l.to_string());
        }
        if let Some(o) = offset {
            sql.push_str(" OFFSET ");
            sql.push_str(&o.to_string());
        }

        (sql, params)
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
    pub fn correlated<T: BaseTable, InnerSel>(
        &self,
        _table: T,
        selection: InnerSel,
    ) -> Correlated<D, Scope, Cons<TableSlot<T, NotNull>, Scope>, InnerSel> {
        Correlated {
            inner: Select {
                body: SelectBody::new(T::NAME, Vec::new()),
                selection,
                _marker: PhantomData,
            },
            _marker: PhantomData,
        }
    }
}

impl<D, Outer, Scope, Sel> Correlated<D, Outer, Scope, Sel> {
    /// Checked against the subquery's own scope, which includes every table
    /// the outer query had — that is what makes it *correlated*.
    pub fn filter<Req, Idxs>(mut self, cond: Expr<Req, Bool>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.inner = self.inner.filter(cond);
        self
    }

    /// AND-folds a runtime-length collection of discharged conditions, the
    /// same way `Select::filter_all` does.
    pub fn filter_all(mut self, conds: impl IntoIterator<Item = Predicate<Scope>>) -> Self {
        self.inner = self.inner.filter_all(conds);
        self
    }

    pub fn group_by<S: SqlType, Req, Idxs>(mut self, key: impl IntoExpr<S, Req = Req>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.inner = self.inner.group_by(key);
        self
    }

    pub fn having<Req, Idxs>(mut self, cond: Expr<Req, Bool>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.inner = self.inner.having(cond);
        self
    }

    pub fn limit(mut self, n: impl crate::row::IntoLimit) -> Self {
        self.inner = self.inner.limit(n);
        self
    }

    pub fn offset(mut self, n: impl crate::row::IntoLimit) -> Self {
        self.inner = self.inner.offset(n);
        self
    }

    pub fn inner_join<New: BaseTable, Req, Idxs>(
        self,
        table: New,
        on: Expr<Req, Bool>,
    ) -> Correlated<D, Outer, Cons<TableSlot<New, NotNull>, Scope>, Sel>
    where
        Cons<TableSlot<New, NotNull>, Scope>: Superset<Req, Idxs>,
    {
        Correlated {
            inner: self.inner.inner_join(table, on),
            _marker: PhantomData,
        }
    }

    pub fn left_join<New: BaseTable, Req, Idxs>(
        self,
        table: New,
        on: Expr<Req, Bool>,
    ) -> Correlated<D, Outer, Cons<TableSlot<New, MaybeNull>, Scope>, Sel>
    where
        Cons<TableSlot<New, MaybeNull>, Scope>: Superset<Req, Idxs>,
    {
        Correlated {
            inner: self.inner.left_join(table, on),
            _marker: PhantomData,
        }
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

fn push_and_list<D: Dialect>(
    sql: &mut String,
    params: &mut Vec<Value>,
    keyword: &str,
    list: &[ExprKind],
) {
    if list.is_empty() {
        return;
    }
    sql.push_str(keyword);
    for (i, e) in list.iter().enumerate() {
        if i > 0 {
            sql.push_str(" AND ");
        }
        render_expr::<D>(e, sql, params);
    }
}

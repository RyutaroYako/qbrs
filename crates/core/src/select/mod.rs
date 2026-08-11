//! The `SELECT` builder: `select(..).from(..).join(..).filter(..)
//! .order_by(..).limit(..)`, in SQL keyword order, argument-based (no
//! turbofish for tables — see the design plan's "surface syntax" section).
//!
//! Split into submodules once this file passed ~750 lines: `selection`
//! (the `Selection` trait deciding what a `.select(..)` list decodes to),
//! `dyn_select` (the `DynSelect` erasure hatch), and `prepared` (reusable
//! named-placeholder statements). All three still need `Select`'s private
//! fields (for `.erase()`/`.prepare()`) or its `render_select_body`/
//! `JoinClause` rendering internals — Rust's privacy rules make a private
//! item visible to the defining module *and all of its descendants*, so
//! those submodules see everything here without needing `pub(crate)`.

use std::marker::PhantomData;

use crate::dialect::{Dialect, SupportsFullOuterJoin, SupportsRightJoin};
use crate::expr::{Bool, Expr, ExprKind, IntoExpr, SqlType, Value};
use crate::render::render_expr;
use crate::scope::{Cons, MapNullable, MaybeNull, Nil, NotNull, Superset, Table, TableSlot};

mod dyn_select;
mod prepared;
mod selection;
mod set_op;

pub use crate::expr::SortDir;
pub use dyn_select::DynSelect;
pub use prepared::{Prepared, PreparedParams, UnresolvedPlaceholder};
pub use selection::Selection;
pub use set_op::SetOp;

/// One `name AS (body)` binding accumulated by `SelectSeed::with` — `body`
/// is already-rendered `?`-placeholder text (see `dialect::RawEmbed`'s doc
/// comment), spliced and renumbered into the final query's own placeholder
/// style at `render_select_body` time, the same mechanism `SetOp` uses for
/// its branches.
struct CteDef {
    name: &'static str,
    column_names: &'static [&'static str],
    sql: String,
    params: Vec<Value>,
}

enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
}

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
    /// Used by `window::Window::order_by` to fold an `OrderKey` into a
    /// window spec's `ORDER BY` list — `window` isn't a descendant module of
    /// `select`, so it can't reach these fields directly the way `dyn_select`/
    /// `prepared`/`set_op` do; `pub(crate)` grants exactly the crate-internal
    /// access needed without making the fields public API.
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

/// Holds just the `SELECT` list until `.from(..)` supplies the first table
/// and therefore the query's initial `Scope`. Splitting this out (rather
/// than requiring `Scope` be known at `.select(..)` time) is what lets the
/// builder read in `SELECT -> FROM -> ...` order while still validating
/// every selected column against the *final* scope only once, at the
/// query's terminal method (`.to_sql()`/`.load()`).
pub struct SelectSeed<Sel> {
    selection: Sel,
    ctes: Vec<CteDef>,
}

pub fn select<Sel>(selection: Sel) -> SelectSeed<Sel> {
    SelectSeed {
        selection,
        ctes: Vec::new(),
    }
}

impl<Sel> SelectSeed<Sel> {
    /// Binds a `WITH name AS (..)` common table expression, built via
    /// `crate::cte::with(name::Table, &inner_query)` — see `cte::CteShape`
    /// for how the inner query's selected columns are checked against
    /// `name`'s `with!{}`-declared shape at compile time. Once bound,
    /// `name::Table` behaves exactly like a real table in `.from()`/
    /// `.inner_join()`/etc: the CTE slots into `Scope`/`Find`/`Superset` the
    /// same way any joined table does, since it really is just another
    /// `scope::Table` impl once its `WITH` binding exists.
    ///
    /// Callable multiple times for multiple independent CTEs, like
    /// `.filter()`. **Known limitation**: a later CTE can't reference an
    /// earlier one bound in the same chain (each `Cte` is rendered
    /// independently at `with()` time, with no visibility into sibling
    /// `.with()` calls) — only `WITH RECURSIVE` and CTE-referencing-CTE are
    /// out of scope for now, not top-level multi-CTE queries themselves.
    pub fn with<D, Marker>(mut self, cte: crate::cte::Cte<D, Marker>) -> Self {
        self.ctes.push(CteDef {
            name: cte.name,
            column_names: cte.column_names,
            sql: cte.sql,
            params: cte.params,
        });
        self
    }

    /// `table` is a value (the zero-sized token the schema macro generates,
    /// e.g. `users::Table`), not a turbofish — `D` and `T` are inferred
    /// from how the resulting `Select` is eventually used (its dialect from
    /// `.load(&db)`, its table from the argument's own type).
    pub fn from<D, T: Table>(self, _table: T) -> Select<D, Cons<TableSlot<T, NotNull>, Nil>, Sel> {
        Select {
            ctes: self.ctes,
            from_table: T::NAME,
            joins: Vec::new(),
            wheres: Vec::new(),
            order_by: Vec::new(),
            group_by: Vec::new(),
            having: Vec::new(),
            limit: None,
            offset: None,
            selection: self.selection,
            _marker: PhantomData,
        }
    }
}

pub struct Select<D, Scope, Sel> {
    ctes: Vec<CteDef>,
    from_table: &'static str,
    joins: Vec<JoinClause>,
    wheres: Vec<ExprKind>,
    order_by: Vec<(ExprKind, SortDir)>,
    group_by: Vec<ExprKind>,
    having: Vec<ExprKind>,
    limit: Option<i64>,
    offset: Option<i64>,
    selection: Sel,
    _marker: PhantomData<fn() -> (D, Scope)>,
}

impl<D, Scope, Sel> Select<D, Scope, Sel> {
    fn retype<NewScope>(self) -> Select<D, NewScope, Sel> {
        Select {
            ctes: self.ctes,
            from_table: self.from_table,
            joins: self.joins,
            wheres: self.wheres,
            order_by: self.order_by,
            group_by: self.group_by,
            having: self.having,
            limit: self.limit,
            offset: self.offset,
            selection: self.selection,
            _marker: PhantomData,
        }
    }

    /// AND-folded, callable any number of times (conditionally, in a loop,
    /// from a shared helper function) without changing `Self`'s type — no
    /// `.$dynamic()`-style escape hatch needed for this, the single most
    /// common "dynamic query" case. See the design plan section 1.
    pub fn filter<Req, Idxs>(mut self, cond: Expr<Req, Bool>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.wheres.push(cond.kind);
        self
    }

    pub fn order_by<Req, Idxs>(mut self, key: OrderKey<Req>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.order_by.push((key.kind, key.dir));
        self
    }

    /// Appends one grouping key; callable multiple times like `.filter()`
    /// (each call adds a column to the `GROUP BY` list, it doesn't replace
    /// it), for the same "dynamic composition without a type change" reason.
    pub fn group_by<S: SqlType, Req, Idxs>(mut self, key: impl IntoExpr<S, Req = Req>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.group_by.push(key.into_expr().kind);
        self
    }

    /// A `WHERE`-shaped filter applied after grouping (aggregate
    /// conditions) — AND-folded across calls exactly like `.filter()`.
    pub fn having<Req, Idxs>(mut self, cond: Expr<Req, Bool>) -> Self
    where
        Scope: Superset<Req, Idxs>,
    {
        self.having.push(cond.kind);
        self
    }

    pub fn limit(mut self, n: i64) -> Self {
        self.limit = Some(n);
        self
    }

    pub fn offset(mut self, n: i64) -> Self {
        self.offset = Some(n);
        self
    }

    /// `New` and `Req` are both inferred from the arguments (the table
    /// token's type, and the `on` expression's own tracked requirement) —
    /// no turbofish, no closure.
    pub fn inner_join<New: Table, Req, Idxs>(
        mut self,
        _table: New,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<New, NotNull>, Scope>, Sel>
    where
        Cons<TableSlot<New, NotNull>, Scope>: Superset<Req, Idxs>,
    {
        self.joins.push(JoinClause {
            kind: JoinKind::Inner,
            table: New::NAME,
            on: on.kind,
        });
        self.retype()
    }

    pub fn left_join<New: Table, Req, Idxs>(
        mut self,
        _table: New,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<New, MaybeNull>, Scope>, Sel>
    where
        Cons<TableSlot<New, MaybeNull>, Scope>: Superset<Req, Idxs>,
    {
        self.joins.push(JoinClause {
            kind: JoinKind::Left,
            table: New::NAME,
            on: on.kind,
        });
        self.retype()
    }

    /// RIGHT JOIN retroactively flips every already-joined table to
    /// nullable (`MapNullable`) before adding the new, guaranteed-present
    /// table — mirrors Drizzle's `AppendToNullabilityMap` rule exactly, see
    /// the design plan section 2.
    pub fn right_join<New: Table, Req, Idxs>(
        mut self,
        _table: New,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<New, NotNull>, Scope::Output>, Sel>
    where
        D: SupportsRightJoin,
        Scope: MapNullable,
        Cons<TableSlot<New, NotNull>, Scope::Output>: Superset<Req, Idxs>,
    {
        self.joins.push(JoinClause {
            kind: JoinKind::Right,
            table: New::NAME,
            on: on.kind,
        });
        self.retype()
    }

    pub fn full_join<New: Table, Req, Idxs>(
        mut self,
        _table: New,
        on: Expr<Req, Bool>,
    ) -> Select<D, Cons<TableSlot<New, MaybeNull>, Scope::Output>, Sel>
    where
        D: SupportsFullOuterJoin,
        Scope: MapNullable,
        Cons<TableSlot<New, MaybeNull>, Scope::Output>: Superset<Req, Idxs>,
    {
        self.joins.push(JoinClause {
            kind: JoinKind::Full,
            table: New::NAME,
            on: on.kind,
        });
        self.retype()
    }
}

impl<D: Dialect, Scope, Sel> Select<D, Scope, Sel> {
    /// The terminal step: this is the *only* point each selected column's
    /// scope-membership is actually checked (proven as a side effect of
    /// `Sel: Selection<Scope, Idx>` type-checking at all — see `Selection`'s
    /// doc comment) — see `SelectSeed`'s docs above for why deferring the
    /// check this far is safe.
    pub fn to_sql<Idx>(&self) -> (String, Vec<Value>)
    where
        Sel: Selection<Scope, Idx>,
    {
        self.render_as::<D, Idx>()
    }

    /// Shared by `to_sql` (rendered in this statement's own dialect `D`),
    /// `exists`/`not_exists`, and `cte::with` (all three via `RawEmbed<D>` —
    /// same identifiers, but `?` placeholders so the fragment can be
    /// renumbered when spliced into an outer query, see `dialect::RawEmbed`'s
    /// doc comment). `pub(crate)` (not private) specifically so `crate::cte`
    /// — a sibling module of `select`, not a descendant — can render a
    /// `Select` down to `(sql, params)` when binding it as a CTE body.
    pub(crate) fn render_as<RD: Dialect, Idx>(&self) -> (String, Vec<Value>)
    where
        Sel: Selection<Scope, Idx>,
    {
        render_select_body::<RD>(
            &self.ctes,
            self.from_table,
            &self.joins,
            &self.selection.exprs(),
            &self.wheres,
            &self.group_by,
            &self.having,
            &self.order_by,
            self.limit,
            self.offset,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn render_select_body<RD: Dialect>(
    ctes: &[CteDef],
    from_table: &str,
    joins: &[JoinClause],
    selection_exprs: &[ExprKind],
    wheres: &[ExprKind],
    group_by: &[ExprKind],
    having: &[ExprKind],
    order_by: &[(ExprKind, SortDir)],
    limit: Option<i64>,
    offset: Option<i64>,
) -> (String, Vec<Value>) {
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
            crate::render::splice_raw::<RD>(&cte.sql, &cte.params, &mut sql, &mut params);
            sql.push(')');
        }
        sql.push(' ');
    }
    sql.push_str("SELECT ");

    for (i, e) in selection_exprs.iter().enumerate() {
        if i > 0 {
            sql.push_str(", ");
        }
        render_expr::<RD>(e, &mut sql, &mut params);
    }

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

impl<D, Scope, Sel> Select<D, Scope, Sel> {
    /// Starts a correlated subquery: a fresh `SELECT` whose scope is
    /// `Cons<TableSlot<T, NotNull>, Scope>` — the new table, prepended onto
    /// *this* (outer) query's entire scope. Because `Find`/`Superset` walk
    /// the whole flat cons-list regardless of where it came from, the
    /// subquery's `.filter()` can reference both its own new table's
    /// columns and any outer column already in `Scope`, with no special
    /// casing — the same mechanism that lets `Select`'s ordinary joins
    /// grow the scope also handles "the new table is a subquery's FROM,
    /// and the existing scope is the outer query's" for free. This is the
    /// exact hypothesis the design plan flagged as unverified; validated
    /// here and in `core/tests/correlated_subquery_smoke.rs`.
    pub fn correlated<T: Table, InnerSel>(
        &self,
        _table: T,
        selection: InnerSel,
    ) -> Select<D, Cons<TableSlot<T, NotNull>, Scope>, InnerSel> {
        Select {
            ctes: Vec::new(),
            from_table: T::NAME,
            joins: Vec::new(),
            wheres: Vec::new(),
            order_by: Vec::new(),
            group_by: Vec::new(),
            having: Vec::new(),
            limit: None,
            offset: None,
            selection,
            _marker: PhantomData,
        }
    }
}

impl<D: Dialect, Scope, Sel> Select<D, Scope, Sel> {
    /// `EXISTS (<this query>)`, usable as a `WHERE`/`HAVING` condition on
    /// whatever outer query this subquery was built from (via
    /// `.correlated()`). `Req = Nil`: the subquery is treated as an opaque,
    /// already-validated fragment (its own `.filter()` calls were already
    /// checked against `Cons<Inner, OuterScope>` when it was built) — the
    /// same trust boundary `sql!{}` uses, not a new one.
    pub fn exists<Idx>(&self) -> Expr<Nil, Bool>
    where
        Sel: Selection<Scope, Idx>,
    {
        let (sql, params) = self.render_as::<crate::dialect::RawEmbed<D>, Idx>();
        Expr::from_kind(ExprKind::Raw {
            text: format!("EXISTS ({sql})"),
            params,
        })
    }

    pub fn not_exists<Idx>(&self) -> Expr<Nil, Bool>
    where
        Sel: Selection<Scope, Idx>,
    {
        let (sql, params) = self.render_as::<crate::dialect::RawEmbed<D>, Idx>();
        Expr::from_kind(ExprKind::Raw {
            text: format!("NOT EXISTS ({sql})"),
            params,
        })
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

//! Window functions: `row_number()`/`rank()`/`dense_rank()` `.over(window()
//! .partition_by(..).order_by(..))`.
//!
//! `WindowFunc<S>` (not `Expr<Req, S>`) is what `row_number()`/`rank()`/
//! `dense_rank()` return — a deliberately narrow type whose *only* method is
//! `.over()`. This is what keeps `.over()` from being callable on an
//! arbitrary `Expr` (e.g. `users::id.over(..)`, which would type-check but
//! render nonsense SQL, or worse, need a runtime check/panic to reject) —
//! the "no compromise between type safety and flexibility" principle this
//! whole crate is built on ruled out a design where `.over()` is generic
//! over any `Expr<Req, S>` and just hopes its `ExprKind` happens to be a
//! bare function call.
//!
//! **Known limitation**: only niladic ranking functions are supported —
//! `sum(col).over(..)`/`avg(col).over(..)` (aggregate functions used as
//! window functions) need the same real function-call design already
//! deferred for `expr::count()` (see its doc comment), since those need to
//! recursively render an inner column reference, not just a literal
//! function-name string. Deferred rather than half-supported by silently
//! falling back to `sql!{}` for that one case.

use std::marker::PhantomData;

use crate::expr::{BigInt, Expr, ExprKind, IntoExpr, SortDir, SqlType};
use crate::scope::{Concat, Nil};
use crate::select::OrderKey;

/// Accumulates a window's `PARTITION BY`/`ORDER BY` lists, tracking `Req`
/// (the tables referenced) via `Concat` across calls exactly the way
/// `Expr::and`/`Expr::or` do — so partitioning or ordering by a column that
/// isn't actually in the query's scope is a `Superset` failure at
/// `.select()`/`.filter()`/etc. time, the same as any other expression.
pub struct Window<Req> {
    partition_by: Vec<ExprKind>,
    order_by: Vec<(ExprKind, SortDir)>,
    _marker: PhantomData<Req>,
}

/// Starts an empty window spec (bare `OVER ()` if never partitioned/ordered
/// — a valid, if unusual, window over the entire result set).
pub fn window() -> Window<Nil> {
    Window {
        partition_by: Vec::new(),
        order_by: Vec::new(),
        _marker: PhantomData,
    }
}

impl<Req> Window<Req> {
    pub fn partition_by<S: SqlType, Req2>(
        self,
        key: impl IntoExpr<S, Req = Req2>,
    ) -> Window<<Req as Concat<Req2>>::Output>
    where
        Req: Concat<Req2>,
    {
        let mut partition_by = self.partition_by;
        partition_by.push(key.into_expr().kind);
        Window {
            partition_by,
            order_by: self.order_by,
            _marker: PhantomData,
        }
    }

    pub fn order_by<Req2>(self, key: OrderKey<Req2>) -> Window<<Req as Concat<Req2>>::Output>
    where
        Req: Concat<Req2>,
    {
        let mut order_by = self.order_by;
        order_by.push(key.into_parts());
        Window {
            partition_by: self.partition_by,
            order_by,
            _marker: PhantomData,
        }
    }
}

/// A bare, argument-free window function reference (`row_number()`,
/// `rank()`, `dense_rank()`) — not yet a usable `Expr`, since a window
/// function has no meaning without an `OVER (..)` clause. See this module's
/// doc comment for why this is a separate type from `Expr` rather than
/// `Expr` itself.
pub struct WindowFunc<S: SqlType> {
    sql: &'static str,
    _marker: PhantomData<S>,
}

impl<S: SqlType> WindowFunc<S> {
    pub fn over<WindowReq>(self, window: Window<WindowReq>) -> Expr<WindowReq, S> {
        Expr::from_kind(ExprKind::Window {
            func: self.sql.to_string(),
            partition_by: window.partition_by,
            order_by: window.order_by,
        })
    }
}

fn window_func<S: SqlType>(sql: &'static str) -> WindowFunc<S> {
    WindowFunc {
        sql,
        _marker: PhantomData,
    }
}

/// `ROW_NUMBER() OVER (..)` — a unique, sequential number per row within its
/// partition, ordered by the window's `ORDER BY`.
pub fn row_number() -> WindowFunc<BigInt> {
    window_func("row_number()")
}

/// `RANK() OVER (..)` — like `row_number()`, but rows tied on the `ORDER BY`
/// key share the same rank, leaving a gap in the sequence afterward.
pub fn rank() -> WindowFunc<BigInt> {
    window_func("rank()")
}

/// `DENSE_RANK() OVER (..)` — like `rank()`, but without the gap after a
/// tie.
pub fn dense_rank() -> WindowFunc<BigInt> {
    window_func("dense_rank()")
}

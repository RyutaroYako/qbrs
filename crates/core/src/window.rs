//! Window functions: `row_number()`/`rank()`/`dense_rank()` `.over(window()
//! .partition_by(..).order_by(..))`.
//!
//! `row_number()`/`rank()`/`dense_rank()` return `WindowFunc<K>`, whose only
//! method is `.over()`, so `.over()` can't be reached from an arbitrary
//! expression that would render nonsense SQL.
//!
//! **Known limitation**: only the ranking functions. An aggregate used as a
//! window function (`sum(col) OVER (..)`) needs `.over()` on the aggregate
//! itself, which is a different builder shape from `WindowFunc`.

use std::marker::PhantomData;

use crate::expr::{BigInt, ExprKind, IntoExpr, Keyed, SortDir};
use crate::scope::{Concat, Nil};
use crate::select::OrderKey;

/// Accumulates a window's `PARTITION BY`/`ORDER BY` lists, growing `Req`
/// via `Concat` as `Expr::and`/`or` do — so partitioning by an out-of-scope
/// column fails the same `Superset` check any other expression would.
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
    pub fn partition_by<Req2>(
        self,
        key: impl IntoExpr<Req = Req2>,
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
/// `rank()`, `dense_rank()`) — not yet a usable expression, since a window
/// function has no meaning without an `OVER (..)` clause. See this module's
/// doc comment for why this is a separate type from `Expr` rather than
/// `Expr` itself.
///
/// `K` is the row key `.over(..)` stamps onto the result, so a selected
/// `row_number()` is readable as `row.row_number()` with nothing declared.
pub struct WindowFunc<K> {
    sql: &'static str,
    _marker: PhantomData<fn() -> K>,
}

impl<K> WindowFunc<K> {
    /// Every ranking function counts rows, so the result is `BigInt` rather
    /// than a parameter — an aggregate over a window, which would have the
    /// aggregate's own type, is the separate shape this module defers.
    pub fn over<WindowReq>(self, window: Window<WindowReq>) -> Keyed<K, WindowReq, BigInt> {
        Keyed::from_kind(ExprKind::Window {
            func: self.sql,
            partition_by: window.partition_by,
            order_by: window.order_by,
        })
    }
}

fn window_func<K>(sql: &'static str) -> WindowFunc<K> {
    WindowFunc {
        sql,
        _marker: PhantomData,
    }
}

crate::row::expr_key!(
    RowNumber,
    HasRowNumber,
    row_number,
    "The identity a selected `row_number() OVER (..)` is filed under in a row.",
    'r',
    'o',
    'w',
    '_',
    'n',
    'u',
    'm',
    'b',
    'e',
    'r'
);
crate::row::expr_key!(
    Rank,
    HasRank,
    rank,
    "The identity a selected `rank() OVER (..)` is filed under in a row.",
    'r',
    'a',
    'n',
    'k'
);
crate::row::expr_key!(
    DenseRank,
    HasDenseRank,
    dense_rank,
    "The identity a selected `dense_rank() OVER (..)` is filed under in a row.",
    'd',
    'e',
    'n',
    's',
    'e',
    '_',
    'r',
    'a',
    'n',
    'k'
);

/// `ROW_NUMBER() OVER (..)` — a unique, sequential number per row within its
/// partition, ordered by the window's `ORDER BY`.
pub fn row_number() -> WindowFunc<RowNumber> {
    window_func("row_number()")
}

/// `RANK() OVER (..)` — like `row_number()`, but rows tied on the `ORDER BY`
/// key share the same rank, leaving a gap in the sequence afterward.
pub fn rank() -> WindowFunc<Rank> {
    window_func("rank()")
}

/// `DENSE_RANK() OVER (..)` — like `rank()`, but without the gap after a
/// tie.
pub fn dense_rank() -> WindowFunc<DenseRank> {
    window_func("dense_rank()")
}

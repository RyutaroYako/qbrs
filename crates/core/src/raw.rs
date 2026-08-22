//! The `sql!{}` escape hatch: a pressure valve for SQL constructs the typed
//! builder doesn't cover yet. `Req` is always `Nil`, so a raw fragment is
//! exempt from scope checking and the caller is trusted to reference real,
//! in-scope columns by name.
//!
//! Values are still bound as real parameters, never spliced as text, so this
//! stays injection-safe; only the SQL *shape* is unchecked.

/// `sql!(SqlType, "lower(name) = ?", "dan")` -> `Expr<Nil, SqlType>`.
/// `?` placeholders are bound positionally, left to right; `??` is a literal
/// `?`, which is how Postgres's `jsonb` operators are reached.
#[macro_export]
macro_rules! sql {
    ($sql_type:ty, $text:expr $(, $arg:expr)* $(,)?) => {{
        $crate::expr::raw_expr::<$sql_type>(
            ::std::string::ToString::to_string($text),
            ::std::vec![$(::std::convert::Into::<$crate::expr::Value>::into($arg)),*],
        )
    }};
}

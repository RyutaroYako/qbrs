//! The `sql!{}` escape hatch: a pressure valve for SQL constructs the typed
//! builder doesn't cover yet. `Req` is always `Nil`, so a raw fragment is
//! exempt from scope checking and the caller is trusted to reference real,
//! in-scope columns by name.
//!
//! The text is a string *literal*, so a fragment's SQL shape is always a
//! compile-time constant of the calling crate — runtime-assembled text
//! cannot become SQL here. Values are bound as real parameters, never
//! spliced as text. Only the shape is unchecked, and only the author writes
//! it.

/// `sql!(SqlType, "lower(name) = ?", "dan")` -> `Expr<Nil, SqlType>`.
/// `?` placeholders are bound positionally, left to right; `??` is a literal
/// `?`, which is how Postgres's `jsonb` operators are reached.
#[macro_export]
macro_rules! sql {
    ($sql_type:ty, $text:literal $(, $arg:expr)* $(,)?) => {{
        $crate::expr::raw_expr::<$sql_type>(
            $text,
            ::std::vec![$(::std::convert::Into::<$crate::expr::Value>::into($arg)),*],
        )
    }};
}

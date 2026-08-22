//! The `sql!{}` escape hatch: a pressure valve for SQL constructs the typed
//! builder doesn't cover yet. `Req` is always `Nil`, so a raw fragment is
//! exempt from scope checking and the caller is trusted to reference real,
//! in-scope columns by name.
//!
//! Values are still bound as real parameters, never spliced as text, so this
//! stays injection-safe; only the SQL *shape* is unchecked.

/// `sql!(SqlType, "lower(name) = ?", "dan")` -> `Expr<Nil, SqlType>`.
/// `?` placeholders are bound positionally, left to right.
#[macro_export]
macro_rules! sql {
    ($sql_type:ty, $text:expr $(, $arg:expr)* $(,)?) => {{
        $crate::expr::Expr::<$crate::scope::Nil, $sql_type>::from_kind(
            $crate::expr::ExprKind::Raw($crate::render::Fragment::new(
                ::std::string::ToString::to_string($text),
                ::std::vec![$(::std::convert::Into::<$crate::expr::Value>::into($arg)),*],
            )),
        )
    }};
}

//! The `sql!{}` escape hatch: a pressure valve for SQL constructs the typed
//! builder doesn't cover yet. Deliberately narrow and explicit rather than
//! trying to parse real SQL syntax (that would just be a worse `sqlx::query!`
//! — see the design plan's surface-syntax discussion for why a
//! SQL-mimicking macro was rejected as a primary API). `Req` is always
//! `Nil`: a raw fragment is exempt from scope checking by design, same as
//! Drizzle's own `sql\`...\`` escape hatch — the caller is trusted to
//! reference real, in-scope columns by name.
//!
//! Values are still bound as real parameters (never spliced as text), so
//! this stays injection-safe; only the SQL *shape* is unchecked.

/// `sql!(SqlType, "lower(name) = ?", "dan")` -> `Expr<Nil, SqlType>`.
/// `?` placeholders are bound positionally, left to right.
#[macro_export]
macro_rules! sql {
    ($sql_type:ty, $text:expr $(, $arg:expr)* $(,)?) => {{
        $crate::expr::Expr::<$crate::scope::Nil, $sql_type>::from_kind(
            $crate::expr::ExprKind::Raw {
                text: ::std::string::ToString::to_string($text),
                params: ::std::vec![$(::std::convert::Into::<$crate::expr::Value>::into($arg)),*],
            },
        )
    }};
}

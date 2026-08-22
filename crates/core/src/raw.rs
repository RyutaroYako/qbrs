//! The `sql!{}` escape hatch: a pressure valve for SQL constructs the typed
//! builder doesn't cover yet.
//!
//! A `?` slot takes a value, which binds as a parameter, or an expression —
//! a column, an aggregate, another fragment — which the renderer writes out
//! itself. A column slot is quoted by the same code that quotes it anywhere
//! else and carries its table into the fragment's `Req`, so
//! `sql!(Numeric, "sum(?)", invoices::amount)` is checked against the
//! query's scope like any other expression. Only the text between the slots
//! is unchecked.
//!
//! That text is a *constant*, so a fragment's SQL shape is always a
//! compile-time constant of the calling crate — runtime-assembled text
//! cannot become SQL here. Every `?` in it is a slot; a literal `?` belongs
//! in a slot's value, since MySQL and SQLite spell their own bind
//! parameters the same way.

/// `sql!(Bool, "lower(?) = ?", users::email, "dan")` -> an `Expr` over
/// whatever tables its slots name. Slots are filled positionally, left to
/// right.
///
/// Every `?` in the text is a slot: there is no escape for a literal one,
/// because two of the three dialects write their own bind parameters as `?`
/// and a `?` left in the text would be read as one. A string containing a
/// `?` goes in a slot, where it binds as a value; Postgres's `jsonb`
/// operators are reached through their function spellings
/// (`jsonb_exists(?, 'key')`).
#[macro_export]
macro_rules! sql {
    ($sql_type:ty, $text:expr $(, $arg:expr)* $(,)?) => {{
        // Binding the text to a `const` first is what rejects a runtime
        // string: `concat!`, `include_str!` and a `const` of your own all
        // pass, an assembled `String` does not.
        const __QBRS_SQL: &'static str = $text;
        // Both counts are constants here, so a mismatch is a compile error
        // rather than a panic when the expression is built.
        const _: () = ::std::assert!(
            $crate::expr::placeholder_count(__QBRS_SQL)
                == <[&'static str]>::len(&[$(::std::stringify!($arg)),*]),
            "`sql!` needs one argument per `?` slot",
        );
        $crate::expr::raw_expr::<$sql_type, _>(__QBRS_SQL, ($($arg,)*))
    }};
}

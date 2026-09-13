//! The `sql!{}` escape hatch: a pressure valve for SQL constructs the typed
//! builder doesn't cover yet.
//!
//! A `?` slot takes a value, which binds as a parameter, or an expression
//! (a column, an aggregate, another fragment), which the renderer writes out
//! itself. A column slot is quoted by the same code that quotes it anywhere
//! else and carries its table into the fragment's `Req`, so
//! `sql!(Numeric, "sum(?)", invoices::amount)` is checked against the
//! query's scope like any other expression. Only the text between the slots
//! is unchecked.
//!
//! One fragment reused across clauses of one statement (selected, grouped
//! by, ordered by) renders as one expression, which is what Postgres's
//! syntactic `GROUP BY` matching asks for. Under Postgres its binds are
//! named again rather than bound again, since `$N` names a parameter. Under
//! MySQL and SQLite `?` *is* the next parameter, so the occurrences read
//! alike and each rebinds, and the statement carries one parameter per
//! occurrence.
//!
//! `sql!` binds that text to a `const` first, so text assembled at runtime
//! cannot reach SQL through this macro. The primitive it expands to,
//! `expr::raw_expr`, takes a bare `&'static str` and has no such guard, so
//! `String::leak` reaches it. It is `#[doc(hidden)]` because `sql!` is the
//! door; a caller who walks around it is writing the unchecked SQL
//! themselves. Every `?` in the text is a slot; a literal `?` belongs in a
//! slot's value, since MySQL and SQLite spell their own bind parameters the
//! same way.

/// `sql!(Bool, "lower(?) = ?", users::email, "dan")` -> a `Declared`
/// expression over whatever tables its slots name. It is keyed as
/// `row::Anon`, so it is selectable but not readable by name until
/// `.label(..)`. Slots are filled positionally, left to right.
///
/// Every `?` in the text is a slot: there is no escape for a literal one,
/// because two of the three dialects write their own bind parameters as `?`
/// and a `?` left in the text would be read as one. A string containing a
/// `?` goes in a slot, where it binds as a value; Postgres's `jsonb`
/// operators are reached through their function spellings
/// (`jsonb_exists(?, 'key')`).
#[macro_export]
macro_rules! sql {
    // The expansion is a *call*, with the const-guard block as its first
    // argument: a block in tail position is coerced to whatever the
    // surrounding expression expects, which made `!sql!(..)` unify against
    // the operand type instead of `Not::Output`.
    ($sql_type:ty, $text:expr $(, $arg:expr)* $(,)?) => {
        $crate::expr::raw_expr::<$sql_type, _>(
            {
                // Binding the text to a `const` first is what rejects a
                // runtime string: `concat!`, `include_str!` and a `const` of
                // your own all pass, an assembled `String` does not. The
                // binding is named for the rule because rustc quotes the
                // line back at whoever breaks it.
                const SQL_TEXT_MUST_BE_A_LITERAL: &'static str = $text;
                // Both counts are constants here, so a mismatch is a compile
                // error rather than a panic when the expression is built.
                const _: () = ::std::assert!(
                    $crate::expr::placeholder_count(SQL_TEXT_MUST_BE_A_LITERAL)
                        == <[&'static str]>::len(&[$(::std::stringify!($arg)),*]),
                    "`sql!` needs one argument per `?` slot",
                );
                SQL_TEXT_MUST_BE_A_LITERAL
            },
            ($($arg,)*),
        )
    };
}

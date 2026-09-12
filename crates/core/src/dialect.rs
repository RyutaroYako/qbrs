//! Dialect markers and capability gating.
//!
//! Dialect-specific capabilities (Postgres `.returning()`, MySQL's lack of
//! it, etc.) are sealed marker traits implemented only for the dialects that
//! actually support them, so calling an unsupported method is a compile
//! error rather than a documentation footnote.
//!
//! Minimum supported version per dialect:
//! - **Postgres**: any currently-supported version.
//! - **MySQL**: 8.0+ (window functions/CTEs).
//! - **SQLite**: 3.39+ (2022-06), for `RIGHT`/`FULL JOIN` support.

mod private {
    pub trait Sealed {}
}

/// `Default` so a dialect can be produced where only its type is known:
/// the render terminals take one as a value (`.to_sql(Postgres)`), which is
/// what lets every builder before them infer it instead of being told.
pub trait Dialect: 'static + Copy + Default + private::Sealed {
    /// SQL identifier quote character, e.g. `"` for Postgres/SQLite, `` ` ``
    /// for MySQL.
    const IDENTIFIER_QUOTE: char;

    /// How this dialect spells the two types an aggregate is cast back to.
    /// `CAST` itself is standard; the type names are not — MySQL takes
    /// `SIGNED` and `DOUBLE` where Postgres takes `BIGINT` and
    /// `DOUBLE PRECISION`.
    const CAST_BIGINT: &'static str = "BIGINT";
    const CAST_DOUBLE: &'static str = "DOUBLE PRECISION";

    /// Whether a set operation's branches may be parenthesised. SQLite's
    /// grammar has no place for a parenthesised `SELECT` around `UNION`.
    const PARENTHESIZED_SET_OP_BRANCHES: bool = true;

    /// How to insert a row that names no column, which is what a table
    /// whose every column is generated leaves. `INSERT INTO t () VALUES ()`
    /// is MySQL's spelling and a syntax error everywhere else.
    const INSERT_NO_COLUMNS: &'static str = " DEFAULT VALUES";

    /// What to put in a `LIMIT` when a query has an `OFFSET` and no limit.
    /// Postgres takes a bare `OFFSET`; SQLite and MySQL don't, and each
    /// spells "no limit" differently.
    const OFFSET_WITHOUT_LIMIT: Option<&'static str> = None;

    /// Whether one bound parameter can be named from several places in a
    /// statement. It follows from how the dialect spells a placeholder:
    /// Postgres's `$N` names a parameter, so repeating `$1` is repeating one
    /// value, while `?` *is* the next parameter, so a second one consumes a
    /// second value. Naming one again keeps a statement's parameters down to
    /// the values it actually holds, at the cost of a rendered text that
    /// depends on which of them are equal: a driver caching prepared
    /// statements by SQL text sees a bulk `INSERT` as one statement per
    /// repetition pattern, as it already sees one per row count.
    const PLACEHOLDERS_ARE_NUMBERED: bool = false;

    /// Writes the placeholder for the `n`th bound parameter (1-indexed).
    /// Postgres numbers them (`$1`, `$2`, ...); MySQL/SQLite are purely
    /// positional (`?` every time, matched by order of appearance).
    fn write_placeholder(n: usize, out: &mut String) {
        let _ = n;
        out.push('?');
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Postgres;
impl private::Sealed for Postgres {}
impl Dialect for Postgres {
    const IDENTIFIER_QUOTE: char = '"';
    const PLACEHOLDERS_ARE_NUMBERED: bool = true;
    fn write_placeholder(n: usize, out: &mut String) {
        use std::fmt::Write as _;
        let _ = write!(out, "${n}");
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MySql;
impl private::Sealed for MySql {}
impl Dialect for MySql {
    const CAST_BIGINT: &'static str = "SIGNED";
    const CAST_DOUBLE: &'static str = "DOUBLE";
    const OFFSET_WITHOUT_LIMIT: Option<&'static str> = Some("18446744073709551615");
    const IDENTIFIER_QUOTE: char = '`';
    const INSERT_NO_COLUMNS: &'static str = " () VALUES ()";
    // Uses the default `?` placeholder.
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Sqlite;
impl private::Sealed for Sqlite {}
impl Dialect for Sqlite {
    const PARENTHESIZED_SET_OP_BRANCHES: bool = false;
    const OFFSET_WITHOUT_LIMIT: Option<&'static str> = Some("-1");
    const IDENTIFIER_QUOTE: char = '"';
    // Uses the default `?` placeholder.
}

/// `RETURNING` support (Postgres, SQLite 3.35+). MySQL has no equivalent
/// SQL construct at any version.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no `RETURNING`",
    label = "Postgres and SQLite do; MySQL has no equivalent at any version",
    note = "read the rows back with a second statement, or write the query for a dialect that has it"
)]
pub trait SupportsReturning: Dialect {}
impl SupportsReturning for Postgres {}
impl SupportsReturning for Sqlite {}

/// `ON CONFLICT DO UPDATE/NOTHING` support (Postgres, SQLite). MySQL's
/// differently-shaped `ON DUPLICATE KEY UPDATE` gets its own capability
/// trait when it lands.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no `ON CONFLICT`",
    label = "Postgres and SQLite do; MySQL spells upsert as `ON DUPLICATE KEY UPDATE`",
    note = "that is a different clause, not a spelling of this one, so it isn't rendered from `.on_conflict_*(..)`"
)]
pub trait SupportsOnConflict: Dialect {}
impl SupportsOnConflict for Postgres {}
impl SupportsOnConflict for Sqlite {}

/// `RIGHT JOIN` support. Every dialect this crate speaks has it (SQLite
/// since 3.39, the minimum this crate targets), so this gate excludes
/// nothing today — it is here so that `SupportsFullOuterJoin`, which MySQL
/// genuinely lacks, is one capability rather than a pair of joins lumped
/// together, and so that adding a dialect without `RIGHT JOIN` is a new
/// impl rather than a change to `right_join`'s signature.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no `RIGHT JOIN`",
    label = "swap the tables and use `.left_join(..)`, which every dialect has"
)]
pub trait SupportsRightJoin: Dialect {}
impl SupportsRightJoin for Postgres {}
impl SupportsRightJoin for MySql {}
impl SupportsRightJoin for Sqlite {}

/// `FULL JOIN` support: Postgres always, SQLite 3.39+, **not** MySQL at any
/// version. The usual MySQL workaround is a `UNION` of `LEFT` and `RIGHT`
/// joins — a different SQL shape, not something `.full_join()` should
/// silently rewrite into.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no `FULL JOIN`",
    label = "Postgres and SQLite do; MySQL's idiom is a `UNION` of a `LEFT` and a `RIGHT` join",
    note = "that rewrite is a different query shape, so `.full_join(..)` doesn't do it silently"
)]
pub trait SupportsFullOuterJoin: Dialect {}
impl SupportsFullOuterJoin for Postgres {}
impl SupportsFullOuterJoin for Sqlite {}

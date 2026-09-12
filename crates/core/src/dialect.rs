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

    /// How this dialect spells "concatenate a group's values, separated",
    /// and with it whether the separator can be a parameter. One value
    /// rather than a name beside a syntax, because the two only make sense
    /// together: `group_concat(x, ', ')` is valid MySQL and means something
    /// else entirely — each row's values run together, not the group's.
    const STRING_AGG: StringAggSyntax = StringAggSyntax::Argument("group_concat");

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

/// Where a `string_agg` separator goes, and so whether it is a value or
/// text. Two dialects take it as an ordinary argument, where it binds like
/// any other value; MySQL's grammar takes a literal after a keyword and
/// rejects a parameter there, which is the only reason this crate ever
/// writes a value into SQL instead of handing it to the driver.
#[derive(Debug, Clone, Copy)]
pub enum StringAggSyntax {
    /// `string_agg(x, $1)` / `group_concat(x, ?)`.
    Argument(&'static str),
    /// `group_concat(x SEPARATOR '...')`, the separator written out and
    /// escaped. `backslash_escapes` says how: MySQL reads a backslash
    /// inside a literal as an escape, so a lone one would carry the closing
    /// quote away and has to be doubled. A session running
    /// `NO_BACKSLASH_ESCAPES` reads the doubled pair as two backslashes —
    /// a separator that isn't the one asked for, though still not a way out
    /// of the literal, since a doubled quote escapes under either mode.
    SeparatorKeyword {
        func: &'static str,
        backslash_escapes: bool,
    },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Postgres;
impl private::Sealed for Postgres {}
impl Dialect for Postgres {
    const IDENTIFIER_QUOTE: char = '"';
    const STRING_AGG: StringAggSyntax = StringAggSyntax::Argument("string_agg");
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
    const STRING_AGG: StringAggSyntax = StringAggSyntax::SeparatorKeyword {
        func: "group_concat",
        backslash_escapes: true,
    };
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

/// A data-modifying statement as a CTE body — `WITH x AS (UPDATE ..
/// RETURNING ..) SELECT ..` — which is Postgres's alone. SQLite and MySQL
/// both take a `SELECT` there and nothing else.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no data-modifying CTE",
    label = "only Postgres takes an `INSERT`/`UPDATE`/`DELETE` as a `WITH` body; the others take a `SELECT`",
    note = "the write and the read it feeds are two statements there, in one transaction"
)]
pub trait SupportsDataModifyingCte: Dialect {}
impl SupportsDataModifyingCte for Postgres {}

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

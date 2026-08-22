//! Execution integration between qbrs's query builder and a real Postgres
//! via `sqlx`. This crate owns only the value-binding and row-decoding glue;
//! query building, SQL rendering, and every compile-time guarantee live in
//! `qbrs-core`, which stays independent of any async runtime or driver.

use qbrs_core::delete::{Delete, DeleteReturning};
use qbrs_core::dialect::Postgres;
use qbrs_core::expr::Value;
use qbrs_core::insert::{Insert, InsertReturning, InsertRow};
use qbrs_core::row::{Row, RowCons, RowNil};
use qbrs_core::scope::Table;
use qbrs_core::select::{DynSelect, Prepared, PreparedParams, Select, Selection, SetOp};
use qbrs_core::update::{Update, UpdateReturning};
use sqlx::Row as _;
use sqlx::postgres::PgRow;

/// Errors from executing a qbrs query against Postgres via `sqlx`. An enum
/// rather than a bare `sqlx::Error` so a qbrs-level misuse is distinguishable
/// from a driver/database error without string-matching a message.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A real error from Postgres or the `sqlx` driver: a failed
    /// connection, constraint violation, decode failure, etc.
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),

    /// A `prepare!{}` placeholder reached execution unresolved: either
    /// `Prepared::resolve()` found no matching field in `Params`, or the
    /// query was executed directly instead of through `.prepare()`.
    #[error(transparent)]
    UnresolvedPlaceholder(#[from] qbrs_core::select::UnresolvedPlaceholder),
}

/// This crate's `Result`: the same shape as `sqlx::Result`, with
/// `qbrs_sqlx::Error` as the fixed error type.
pub type Result<T> = std::result::Result<T, Error>;

/// Every extension trait that puts a terminal method on a builder, plus the
/// error type a caller's own signatures have to name. `Result` is
/// deliberately absent: a glob-imported alias of that name shadows
/// `std::result::Result` in every module that follows, and a service layer
/// has its own error type in most of them — write `qbrs_sqlx::Result<T>`
/// where the alias is wanted. Which trait applies
/// depends on the builder, so importing them one at a time is bookkeeping
/// with no decision in it — and `count` in particular resolves against
/// `Iterator::count` with a confusing message until `CountExt` is in scope.
pub mod prelude {
    pub use crate::Error;
    pub use crate::{CountExt, ExecuteExt, LoadExt, PreparedExt};
}

/// Binds a `Value` to a Postgres query parameter. `Value`'s typed `NullX`
/// variants carry the parameter type a NULL bind still has to declare.
///
/// Fallible only for `Value::Placeholder`: an unresolved named placeholder
/// is a misuse no compile-time check here can catch, so it surfaces as
/// `Error::UnresolvedPlaceholder` rather than a wrong bind or a panic.
fn bind_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    v: Value,
) -> Result<sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>> {
    Ok(match v {
        Value::I32(x) => query.bind(x),
        Value::I64(x) => query.bind(x),
        Value::F64(x) => query.bind(x),
        Value::Text(x) => query.bind(x),
        Value::Bool(x) => query.bind(x),
        Value::Bytes(x) => query.bind(x),
        Value::NullI32 => query.bind(None::<i32>),
        Value::NullI64 => query.bind(None::<i64>),
        Value::NullF64 => query.bind(None::<f64>),
        Value::NullText => query.bind(None::<String>),
        Value::NullBool => query.bind(None::<bool>),
        Value::NullBytes => query.bind(None::<Vec<u8>>),
        Value::Placeholder(name) => {
            return Err(qbrs_core::select::UnresolvedPlaceholder(name).into());
        }
    })
}

fn bind_all<'q>(
    mut query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    params: Vec<Value>,
) -> Result<sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>> {
    for p in params {
        query = bind_value(query, p)?;
    }
    Ok(query)
}

/// Generic over `E: sqlx::PgExecutor` so every `.load()`/`.execute()` works
/// against a `&PgPool` or a transaction alike. sqlx implements `Executor` for
/// `&mut PgConnection`, not `Transaction`, so callers pass `&mut *tx`.
async fn fetch_all<'e, T: DecodeRow, E: sqlx::PgExecutor<'e>>(
    executor: E,
    sql: &str,
    params: Vec<Value>,
) -> Result<Vec<T>> {
    let rows = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql)), params)?
        .fetch_all(executor)
        .await?;
    rows.iter()
        .map(|row| T::decode_at(row, &mut 0).map_err(Error::from))
        .collect()
}

async fn fetch_optional<'e, T: DecodeRow, E: sqlx::PgExecutor<'e>>(
    executor: E,
    sql: &str,
    params: Vec<Value>,
) -> Result<Option<T>> {
    let row = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql)), params)?
        .fetch_optional(executor)
        .await?;
    row.as_ref()
        .map(|r| T::decode_at(r, &mut 0).map_err(Error::from))
        .transpose()
}

async fn execute_only<'e, E: sqlx::PgExecutor<'e>>(
    executor: E,
    sql: &str,
    params: Vec<Value>,
) -> Result<u64> {
    let result = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql)), params)?
        .execute(executor)
        .await?;
    Ok(result.rows_affected())
}

/// Everything that produces rows: a `SELECT`, a `RETURNING` clause, an
/// erased `DynSelect`, a `UNION` chain. One trait for all of them keeps the
/// terminal vocabulary tied to what a statement yields rather than to which
/// builder happens to be in hand — `load` for the rows, `load_one` for the
/// first of them, and `ExecuteExt::execute` where there are none to decode.
///
/// `Idx` is threaded through the trait's parameter list for the reason
/// `scope::Superset` explains. Callers never see it; it's inferred.
pub trait LoadExt<Idx> {
    type Output: DecodeRow;

    #[doc(hidden)]
    fn rendered(&self) -> (String, Vec<Value>);

    fn load<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Vec<Self::Output>>> {
        let (sql, params) = self.rendered();
        async move { fetch_all::<Self::Output, E>(executor, &sql, params).await }
    }

    fn load_one<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Option<Self::Output>>> {
        let (sql, params) = self.rendered();
        async move { fetch_optional::<Self::Output, E>(executor, &sql, params).await }
    }
}

impl<Scope, Sel, Idx> LoadExt<Idx> for Select<Postgres, Scope, Sel>
where
    Sel: Selection<Scope, Idx>,
    Sel::Output: DecodeRow,
{
    type Output = Sel::Output;

    fn rendered(&self) -> (String, Vec<Value>) {
        self.to_sql::<Idx>()
    }
}

/// `SELECT count(*)` over a query's `FROM`/`JOIN`/`WHERE`/`GROUP BY`, with
/// its `ORDER BY`/`LIMIT`/`OFFSET` dropped — a total counts the rows that
/// match, not the page being shown. Returns a number rather than an
/// `Option`, since a count query always produces exactly one row.
pub trait CountExt<Idx> {
    fn count<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<i64>>;
}

impl<Scope, Sel: Selection<Scope, Idx>, Idx> CountExt<Idx> for Select<Postgres, Scope, Sel> {
    async fn count<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<i64> {
        count_rows(executor, self.count_sql::<Idx>()).await
    }
}

/// Erasure is for a query whose joins depend on a condition, and such a
/// query is paged like any other, so it counts like any other.
impl<Output> CountExt<()> for DynSelect<Postgres, Output> {
    async fn count<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<i64> {
        count_rows(executor, self.count_sql()).await
    }
}

async fn count_rows<'e, E: sqlx::PgExecutor<'e>>(
    executor: E,
    (sql, params): (String, Vec<Value>),
) -> Result<i64> {
    let row = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql.as_str())), params)?
        .fetch_one(executor)
        .await?;
    Ok(row.try_get::<i64, _>(0)?)
}

pub trait ExecuteExt {
    fn execute<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<u64>>;
}

impl<T: Table, R: InsertRow<Table = T>> ExecuteExt for Insert<Postgres, T, R> {
    async fn execute<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<u64> {
        let (sql, params) = self.to_sql();
        execute_only(executor, &sql, params).await
    }
}

impl<T: Table> ExecuteExt for Update<Postgres, T> {
    async fn execute<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<u64> {
        let (sql, params) = self.to_sql();
        execute_only(executor, &sql, params).await
    }
}

impl<T: Table> ExecuteExt for Delete<Postgres, T> {
    async fn execute<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<u64> {
        let (sql, params) = self.to_sql();
        execute_only(executor, &sql, params).await
    }
}

/// The scope a `RETURNING` clause is checked against: just the table being
/// written to.
type TableScope<T> = qbrs_core::scope::Cons<
    qbrs_core::scope::TableSlot<T, qbrs_core::scope::NotNull>,
    qbrs_core::scope::Nil,
>;

impl<T: Table, R: InsertRow<Table = T>, Sel, Idx> LoadExt<Idx>
    for InsertReturning<Postgres, T, R, Sel>
where
    Sel: Selection<TableScope<T>, Idx>,
    Sel::Output: DecodeRow,
{
    type Output = Sel::Output;
    fn rendered(&self) -> (String, Vec<Value>) {
        self.to_sql()
    }
}

impl<T: Table, Sel, Idx> LoadExt<Idx> for UpdateReturning<Postgres, T, Sel>
where
    Sel: Selection<TableScope<T>, Idx>,
    Sel::Output: DecodeRow,
{
    type Output = Sel::Output;
    fn rendered(&self) -> (String, Vec<Value>) {
        self.to_sql()
    }
}

impl<T: Table, Sel, Idx> LoadExt<Idx> for DeleteReturning<Postgres, T, Sel>
where
    Sel: Selection<TableScope<T>, Idx>,
    Sel::Output: DecodeRow,
{
    type Output = Sel::Output;
    fn rendered(&self) -> (String, Vec<Value>) {
        self.to_sql()
    }
}

/// Decodes a `DynSelect`'s `Output` positionally out of a `PgRow`. Narrower
/// than `PgDecode`: erasure leaves only the plain-Rust `Output`, with no
/// `Selection` impl left to hang decoding off, so this is implemented
/// directly against the same closed set of native types.
pub trait DecodeRow: Sized {
    #[doc(hidden)]
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self>;
}

macro_rules! decode_row_leaf {
    ($ty:ty) => {
        impl DecodeRow for $ty {
            fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self> {
                let v = row.try_get::<$ty, _>(*idx)?;
                *idx += 1;
                Ok(v)
            }
        }
        impl DecodeRow for Option<$ty> {
            fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self> {
                let v = row.try_get::<Option<$ty>, _>(*idx)?;
                *idx += 1;
                Ok(v)
            }
        }
    };
}
decode_row_leaf!(i32);
decode_row_leaf!(i64);
decode_row_leaf!(f64);
decode_row_leaf!(String);
decode_row_leaf!(bool);
decode_row_leaf!(Vec<u8>);

impl DecodeRow for RowNil {
    fn decode_at(_row: &PgRow, _idx: &mut usize) -> sqlx::Result<Self> {
        Ok(RowNil)
    }
}

impl<K, V: DecodeRow, Tail: DecodeRow> DecodeRow for RowCons<K, V, Tail> {
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self> {
        let value = V::decode_at(row, idx)?;
        Ok(RowCons::new(value, Tail::decode_at(row, idx)?))
    }
}

impl<L: DecodeRow> DecodeRow for Row<L> {
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self> {
        Ok(Row::new(L::decode_at(row, idx)?))
    }
}

/// An erased query and a set-op chain were both rendered before their
/// selection type was gone, leaving nothing for `Idx` to index — hence
/// `LoadExt<()>`, the same trait with an empty proof.
impl<Output: DecodeRow> LoadExt<()> for DynSelect<Postgres, Output> {
    type Output = Output;
    fn rendered(&self) -> (String, Vec<Value>) {
        self.to_sql()
    }
}

impl<Output: DecodeRow> LoadExt<()> for SetOp<Postgres, Output> {
    type Output = Output;
    fn rendered(&self) -> (String, Vec<Value>) {
        self.to_sql()
    }
}

/// Runs a `prepare!{}`-built query, resolving its named placeholders from
/// `params` first. Separate from `LoadExt` only because the values arrive at
/// the call rather than being baked into the query: one `Prepared` is meant
/// to serve many calls, and `.resolve()` clones the template rather than
/// re-rendering it.
pub trait PreparedExt<Params> {
    type Output;
    fn load<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
        params: Params,
    ) -> impl std::future::Future<Output = Result<Vec<Self::Output>>>;
    fn load_one<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
        params: Params,
    ) -> impl std::future::Future<Output = Result<Option<Self::Output>>>;
}

impl<Params: PreparedParams, Output: DecodeRow> PreparedExt<Params> for Prepared<Params, Output> {
    type Output = Output;

    async fn load<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
        params: Params,
    ) -> Result<Vec<Output>> {
        let (sql, values) = self.resolve(params)?;
        fetch_all::<Output, E>(executor, &sql, values).await
    }

    async fn load_one<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
        params: Params,
    ) -> Result<Option<Output>> {
        let (sql, values) = self.resolve(params)?;
        fetch_optional::<Output, E>(executor, &sql, values).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // No real Postgres needed: binding inspects the `Value` enum before
    // anything reaches the network, so an unresolved placeholder is
    // reachable, and its error checkable, without a live DB.
    #[test]
    fn unresolved_placeholder_is_a_typed_error_not_a_sqlx_configuration_string() {
        let query = sqlx::query(sqlx::AssertSqlSafe("SELECT $1"));
        let err = match bind_all(query, vec![Value::Placeholder("email")]) {
            Err(e) => e,
            Ok(_) => panic!("unresolved placeholder must fail to bind"),
        };

        assert!(matches!(
            err,
            Error::UnresolvedPlaceholder(qbrs_core::select::UnresolvedPlaceholder("email"))
        ));
        // A real `std::error::Error`, so it composes with
        // `anyhow`/`Box<dyn Error>`.
        let _: &dyn std::error::Error = &err;
        assert_eq!(err.to_string(), "no value provided for placeholder `email`");
    }

    #[test]
    fn sqlx_errors_convert_via_from() {
        let err: Error = sqlx::Error::RowNotFound.into();
        assert!(matches!(err, Error::Sqlx(sqlx::Error::RowNotFound)));
    }
}

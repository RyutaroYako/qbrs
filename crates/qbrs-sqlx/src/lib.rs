//! Execution integration between qbrs's query builder and a real Postgres
//! via `sqlx`. This crate owns only the value-binding and row-decoding glue;
//! query building, SQL rendering, and every compile-time guarantee live in
//! `qbrs-core`, which stays independent of any async runtime or driver.

use qbrs_core::delete::{Delete, DeleteReturning};
use qbrs_core::dialect::Postgres;
use qbrs_core::expr::{Aliased, Column, ColumnKey, Expr, Keyed, SqlType, Value};
use qbrs_core::insert::{Insert, InsertReturning, InsertRow};
use qbrs_core::row::{Row, RowCons, RowNil};
use qbrs_core::scope::Table;
use qbrs_core::select::{DynSelect, Prepared, PreparedParams, RowField, Select, Selection, SetOp};
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

/// Every extension trait that puts a `.load()`/`.execute()` on a builder.
/// Which one applies depends on the builder, so importing them one at a
/// time is bookkeeping with no decision in it.
pub mod prelude {
    pub use crate::{ExecuteExt, LoadDynExt, LoadExt, LoadReturningExt, LoadSetOpExt, PreparedExt};
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

/// Decodes one selected item positionally out of a `PgRow`. Separate from
/// `RowField` so `qbrs-core` never depends on `sqlx`: each impl adds a
/// `Decode`/`Type` bound to an existing `RowField` impl. Join-derived
/// `Option<T>` needs no special handling — `RowField::Value` already
/// resolved it, and sqlx decodes `Option<T>` for free.
pub trait PgDecodeField<Scope, Idx>: RowField<Scope, Idx> {
    #[doc(hidden)]
    fn decode_field(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self::Value>;
}

macro_rules! decode_field {
    (impl[$($generics:tt)*] $ty:ty) => {
        impl<$($generics)*, Scope, Idx> PgDecodeField<Scope, Idx> for $ty
        where
            $ty: RowField<Scope, Idx>,
            <$ty as RowField<Scope, Idx>>::Value:
                for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
        {
            fn decode_field(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self::Value> {
                let v = row.try_get::<Self::Value, _>(*idx)?;
                *idx += 1;
                Ok(v)
            }
        }
    };
}
decode_field!(impl[C: ColumnKey] Column<C>);
decode_field!(impl[Req, S: SqlType] Expr<Req, S>);
decode_field!(impl[K, Req, S: SqlType] Keyed<K, Req, S>);
decode_field!(impl[K, Inner] Aliased<K, Inner>);

/// Decodes a whole `Selection<Scope, Idx>` out of a `PgRow`, positionally.
pub trait PgDecode<Scope, Idx>: Selection<Scope, Idx> {
    #[doc(hidden)]
    fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self::Output>;
}

macro_rules! decode_scalar {
    (impl[$($generics:tt)*] $ty:ty) => {
        impl<$($generics)*, Scope, Idx> PgDecode<Scope, Idx> for $ty
        where
            $ty: Selection<Scope, Idx> + PgDecodeField<Scope, Idx>,
            $ty: Selection<Scope, Idx, Output = <$ty as RowField<Scope, Idx>>::Value>,
        {
            fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self::Output> {
                <$ty as PgDecodeField<Scope, Idx>>::decode_field(row, idx)
            }
        }
    };
}
decode_scalar!(impl[C: ColumnKey] Column<C>);
decode_scalar!(impl[Req, S: SqlType] Expr<Req, S>);
decode_scalar!(impl[K, Req, S: SqlType] Keyed<K, Req, S>);
decode_scalar!(impl[K, Inner] Aliased<K, Inner>);

macro_rules! decode_row_chain {
    ($row:ident, $idx:ident, $n:ident $i:ident) => {
        RowCons::new(<$n as PgDecodeField<Scope, $i>>::decode_field($row, $idx)?, RowNil)
    };
    ($row:ident, $idx:ident, $n:ident $i:ident, $($rest:tt)*) => {
        RowCons::new(
            <$n as PgDecodeField<Scope, $i>>::decode_field($row, $idx)?,
            decode_row_chain!($row, $idx, $($rest)*),
        )
    };
}

macro_rules! decode_tuple {
    ($($n:ident $i:ident),+) => {
        impl<Scope, $($n,)+ $($i,)+> PgDecode<Scope, ($($i,)+)> for ($($n,)+)
        where
            $($n: PgDecodeField<Scope, $i>,)+
        {
            fn decode_at(row: &PgRow, idx: &mut usize) -> sqlx::Result<Self::Output> {
                Ok(Row::new(decode_row_chain!(row, idx, $($n $i),+)))
            }
        }
    };
}
decode_tuple!(A IA);
decode_tuple!(A IA, B IB);
decode_tuple!(A IA, B IB, C IC);
decode_tuple!(A IA, B IB, C IC, D ID);
decode_tuple!(A IA, B IB, C IC, D ID, E IE);
decode_tuple!(A IA, B IB, C IC, D ID, E IE, F IF);
decode_tuple!(A IA, B IB, C IC, D ID, E IE, F IF, G IG);
decode_tuple!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH);
decode_tuple!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II);
decode_tuple!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ);
decode_tuple!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK);
decode_tuple!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL);
decode_tuple!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM);
decode_tuple!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN);
decode_tuple!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO);
decode_tuple!(A IA, B IB, C IC, D ID, E IE, F IF, G IG, H IH, I II, J IJ, K IK, L IL, M IM, N IN, O IO, P IP);

/// Generic over `E: sqlx::PgExecutor` so every `.load()`/`.execute()` works
/// against a `&PgPool` or a transaction alike. sqlx implements `Executor` for
/// `&mut PgConnection`, not `Transaction`, so callers pass `&mut *tx`.
async fn fetch_all<'e, Scope, Idx, Sel: PgDecode<Scope, Idx>, E: sqlx::PgExecutor<'e>>(
    executor: E,
    sql: &str,
    params: Vec<Value>,
) -> Result<Vec<Sel::Output>> {
    let rows = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql)), params)?
        .fetch_all(executor)
        .await?;
    rows.iter()
        .map(|row| Sel::decode_at(row, &mut 0).map_err(Error::from))
        .collect()
}

async fn fetch_optional<'e, Scope, Idx, Sel: PgDecode<Scope, Idx>, E: sqlx::PgExecutor<'e>>(
    executor: E,
    sql: &str,
    params: Vec<Value>,
) -> Result<Option<Sel::Output>> {
    let row = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql)), params)?
        .fetch_optional(executor)
        .await?;
    row.as_ref()
        .map(|r| Sel::decode_at(r, &mut 0).map_err(Error::from))
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

/// `Idx` is threaded through the trait's parameter list for the reason
/// `scope::Superset` explains. Callers never see it; it's inferred.
pub trait LoadExt<Idx> {
    type Output;
    fn load<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Vec<Self::Output>>>;
    fn load_one<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Option<Self::Output>>>;
}

impl<Scope, Sel: PgDecode<Scope, Idx>, Idx> LoadExt<Idx> for Select<Postgres, Scope, Sel> {
    type Output = Sel::Output;

    async fn load<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<Vec<Self::Output>> {
        let (sql, params) = self.to_sql::<Idx>();
        fetch_all::<Scope, Idx, Sel, E>(executor, &sql, params).await
    }

    async fn load_one<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> Result<Option<Self::Output>> {
        let (sql, params) = self.to_sql::<Idx>();
        fetch_optional::<Scope, Idx, Sel, E>(executor, &sql, params).await
    }
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

pub trait LoadReturningExt<Idx> {
    type Output;
    fn load<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Vec<Self::Output>>>;
}

impl<T: Table, R: InsertRow<Table = T>, Sel, Idx> LoadReturningExt<Idx>
    for InsertReturning<Postgres, T, R, Sel>
where
    Sel: PgDecode<
            qbrs_core::scope::Cons<
                qbrs_core::scope::TableSlot<T, qbrs_core::scope::NotNull>,
                qbrs_core::scope::Nil,
            >,
            Idx,
        >,
{
    type Output = Sel::Output;
    async fn load<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<Vec<Self::Output>> {
        let (sql, params) = self.to_sql();
        fetch_all::<
            qbrs_core::scope::Cons<
                qbrs_core::scope::TableSlot<T, qbrs_core::scope::NotNull>,
                qbrs_core::scope::Nil,
            >,
            Idx,
            Sel,
            E,
        >(executor, &sql, params)
        .await
    }
}

impl<T: Table, Sel, Idx> LoadReturningExt<Idx> for UpdateReturning<Postgres, T, Sel>
where
    Sel: PgDecode<
            qbrs_core::scope::Cons<
                qbrs_core::scope::TableSlot<T, qbrs_core::scope::NotNull>,
                qbrs_core::scope::Nil,
            >,
            Idx,
        >,
{
    type Output = Sel::Output;
    async fn load<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<Vec<Self::Output>> {
        let (sql, params) = self.to_sql();
        fetch_all::<
            qbrs_core::scope::Cons<
                qbrs_core::scope::TableSlot<T, qbrs_core::scope::NotNull>,
                qbrs_core::scope::Nil,
            >,
            Idx,
            Sel,
            E,
        >(executor, &sql, params)
        .await
    }
}

impl<T: Table, Sel, Idx> LoadReturningExt<Idx> for DeleteReturning<Postgres, T, Sel>
where
    Sel: PgDecode<
            qbrs_core::scope::Cons<
                qbrs_core::scope::TableSlot<T, qbrs_core::scope::NotNull>,
                qbrs_core::scope::Nil,
            >,
            Idx,
        >,
{
    type Output = Sel::Output;
    async fn load<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<Vec<Self::Output>> {
        let (sql, params) = self.to_sql();
        fetch_all::<
            qbrs_core::scope::Cons<
                qbrs_core::scope::TableSlot<T, qbrs_core::scope::NotNull>,
                qbrs_core::scope::Nil,
            >,
            Idx,
            Sel,
            E,
        >(executor, &sql, params)
        .await
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

pub trait LoadDynExt {
    type Output;
    fn load<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Vec<Self::Output>>>;
}

impl<Output: DecodeRow> LoadDynExt for DynSelect<Postgres, Output> {
    type Output = Output;
    async fn load<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<Vec<Output>> {
        let (sql, params) = self.to_sql();
        let rows = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql.as_str())), params)?
            .fetch_all(executor)
            .await?;
        rows.iter()
            .map(|row| Output::decode_at(row, &mut 0).map_err(Error::from))
            .collect()
    }
}

/// Loads a `UNION`/`INTERSECT`/`EXCEPT` chain. Reuses `DecodeRow` for the
/// same reason `LoadDynExt` does: a `SetOp`'s branches were rendered to
/// fragments at combine time, leaving only the plain `Output` to decode
/// against.
pub trait LoadSetOpExt {
    type Output;
    fn load<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
    ) -> impl std::future::Future<Output = Result<Vec<Self::Output>>>;
}

impl<Output: DecodeRow> LoadSetOpExt for SetOp<Postgres, Output> {
    type Output = Output;
    async fn load<'e, E: sqlx::PgExecutor<'e>>(&self, executor: E) -> Result<Vec<Output>> {
        let (sql, params) = self.to_sql();
        let rows = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql.as_str())), params)?
            .fetch_all(executor)
            .await?;
        rows.iter()
            .map(|row| Output::decode_at(row, &mut 0).map_err(Error::from))
            .collect()
    }
}

/// Executes a `prepare!{}`-built query, resolving its named placeholders
/// from `params` first. One `Prepared` is meant to serve many `execute`
/// calls: `.resolve()` clones the template rather than re-rendering it.
pub trait PreparedExt<Params> {
    type Output;
    fn execute<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
        params: Params,
    ) -> impl std::future::Future<Output = Result<Vec<Self::Output>>>;
}

impl<Params: PreparedParams, Output: DecodeRow> PreparedExt<Params> for Prepared<Params, Output> {
    type Output = Output;
    async fn execute<'e, E: sqlx::PgExecutor<'e>>(
        &self,
        executor: E,
        params: Params,
    ) -> Result<Vec<Output>> {
        let (sql, values) = self.resolve(params)?;
        let rows = bind_all(sqlx::query(sqlx::AssertSqlSafe(sql.as_str())), values)?
            .fetch_all(executor)
            .await?;
        rows.iter()
            .map(|row| Output::decode_at(row, &mut 0).map_err(Error::from))
            .collect()
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

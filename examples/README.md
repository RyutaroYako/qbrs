# qbrs examples

Runnable, end-to-end examples demonstrating qbrs against a real Postgres.
Each one is a standalone `cargo run` target under [`examples/`](examples).

## Setup

None. Just run one:

```sh
cargo run -p qbrs-examples --example 01_select_basic
```

Each example starts its own throwaway PostgreSQL 17.5, embedded via
[`pglite-rs`](https://crates.io/crates/pglite-rs) — the `postgres-pglite`
engine linked into the binary and served over a unix socket, so `sqlx`
connects to it exactly as it would to any Postgres. No Docker, no Postgres
install, and nothing fetched at run time; the server is torn down when the
example exits.

That teardown is why `setup_db()` hands back a second value:

```rust
let (pool, _db) = setup_db().await;
```

`_db` owns the server, so it has to stay bound for the body of the example —
dropping it early would stop Postgres out from under the pool.

To run against an external Postgres instead, set `DATABASE_URL`:

```sh
DATABASE_URL=postgres://postgres:postgres@localhost:5432/qbrs_test \
  cargo run -p qbrs-examples --example 01_select_basic
```

## Examples

| Example | Demonstrates |
|---|---|
| [`01_select_basic`](examples/01_select_basic.rs) | `SELECT ... WHERE ... ORDER BY ... LIMIT` |
| [`02_select_join`](examples/02_select_join.rs) | LEFT vs. INNER JOIN — automatically-derived `Option<T>` nullability |
| [`03_insert`](examples/03_insert.rs) | `Defaultable<T>`, bulk insert, `RETURNING` |
| [`04_update`](examples/04_update.rs) | partial updates, `Option<Option<T>>` for "set NULL" vs. "don't touch" |
| [`05_delete`](examples/05_delete.rs) | `DELETE ... RETURNING` |
| [`06_raw_sql`](examples/06_raw_sql.rs) | the `sql!{}` escape hatch |
| [`07_dynamic_filters`](examples/07_dynamic_filters.rs) | dynamic query composition without a `.$dynamic()`-style escape hatch |
| [`08_right_full_join`](examples/08_right_full_join.rs) | RIGHT/FULL JOIN — previously-joined tables retroactively become nullable |
| [`09_dynamic_join`](examples/09_dynamic_join.rs) | `DynSelect` — the narrow erasure hatch for conditionally joining or not |
| [`10_prepared`](examples/10_prepared.rs) | `prepare!{}` — a query rendered once, reused across many typed `.load(.., params)` calls |
| [`11_upsert`](examples/11_upsert.rs) | `ON CONFLICT (..) DO NOTHING` / `DO UPDATE SET ..`, gated to dialects with `SupportsOnConflict` |
| [`12_union`](examples/12_union.rs) | `UNION ALL`/`INTERSECT` across `SELECT`s with unrelated `Scope`s |
| [`13_window`](examples/13_window.rs) | `row_number()`/`rank()` `.over(window()..)` |
| [`14_cte`](examples/14_cte.rs) | `with!{}` + `cte::with(..)` + `.inner_join(..)` — a CTE used as a real table |
| [`15_transaction`](examples/15_transaction.rs) | commit/rollback — every `.load()`/`.execute()` is generic over `sqlx::PgExecutor` |
| [`16_row_access`](examples/16_row_access.rs) | rows keyed by column — adding a column moves nothing, and a helper can require just one |
| [`17_from_row`](examples/17_from_row.rs) | `#[derive(FromRow)]` — filling a plain domain struct by field name, `select(users::All)` for a whole table, and `take` when a name doesn't line up |
| [`18_correlated_exists`](examples/18_correlated_exists.rs) | `EXISTS`/`NOT EXISTS` over a subquery built from the outer query, tagged with the tables it references |

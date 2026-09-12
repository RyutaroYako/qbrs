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
connects to it exactly as it would to any Postgres. No Docker and no Postgres
install. The engine is downloaded once, when the crate is first built, and
cached under `~/.cache/pglite-rs`, so nothing is fetched while an example runs;
the server is torn down when the example exits.

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
| [`04_update`](examples/04_update.rs) | partial updates from a request, `.column_null()` for "set NULL" vs. an absent field for "don't touch", `null::<Text>()` as an assignment |
| [`05_delete`](examples/05_delete.rs) | `DELETE ... RETURNING` |
| [`06_raw_sql`](examples/06_raw_sql.rs) | the `sql!{}` escape hatch |
| [`07_dynamic_filters`](examples/07_dynamic_filters.rs) | dynamic query composition without a `.$dynamic()`-style escape hatch |
| [`08_right_full_join`](examples/08_right_full_join.rs) | RIGHT/FULL JOIN — previously-joined tables retroactively become nullable |
| [`09_dynamic_join`](examples/09_dynamic_join.rs) | `DynSelect` — the narrow erasure hatch for conditionally joining or not |
| [`10_prepared`](examples/10_prepared.rs) | `prepare!{}` — a query rendered once, reused across many typed `.load(.., params)` calls |
| [`11_upsert`](examples/11_upsert.rs) | `ON CONFLICT (..) DO NOTHING` / `DO UPDATE SET ..`, gated to dialects with `SupportsOnConflict`, and `partial_index(..)` for a partial unique index |
| [`12_union`](examples/12_union.rs) | `UNION ALL`/`INTERSECT` across `SELECT`s with unrelated `Scope`s |
| [`13_window`](examples/13_window.rs) | `row_number()`/`rank()` `.over(window()..)` |
| [`14_cte`](examples/14_cte.rs) | `with!{}` + `cte::with(..)` + `.inner_join(..)` — a CTE used as a real table |
| [`15_transaction`](examples/15_transaction.rs) | commit/rollback — every `.load()`/`.execute()` is generic over `sqlx::PgExecutor` |
| [`16_row_access`](examples/16_row_access.rs) | rows keyed by column — adding a column moves nothing, and a helper can require just one |
| [`17_from_row`](examples/17_from_row.rs) | `#[derive(FromRow)]` — filling a plain domain struct by field name, `select(users::All)` for a whole table, `from = <column>` where two tables' `id`s collide, and `take` when a name doesn't line up |
| [`18_correlated_exists`](examples/18_correlated_exists.rs) | `EXISTS`/`NOT EXISTS` over a subquery built from the outer query, tagged with the tables it references |
| [`19_dynamic_sort`](examples/19_dynamic_sort.rs) | `sort_key(..)`/`.order_by_all(..)` and `grouping(..)`/`.group_by_all(..)` — a `?sort=` parameter whose keys name different tables |
| [`20_in_subquery`](examples/20_in_subquery.rs) | `IN`/`NOT IN (<subquery>)` via `Select::contains`/`.not_contains` — a subquery's single selected column checked against the outer expression like `.eq(..)` checks two columns |
| [`21_self_join_via_cte`](examples/21_self_join_via_cte.rs) | self-join workaround: a `with!{}` pseudo-table bound to a plain `SELECT` over the same table, joined back to it — qbrs has no table aliasing (see README) |
| [`22_distinct_order_by_selected`](examples/22_distinct_order_by_selected.rs) | `.order_by_selected(..)`/`.order_by_selection(..)` — a `SELECT DISTINCT` sort key checked against the selection at compile time |
| [`23_aggregates`](examples/23_aggregates.rs) | `count()`/`count_of()`/`sum()`/`min()`/`max()`/`avg()`/`string_agg()` — an aggregate keyed by the column it aggregates, and the two questions `count` answers |
| [`24_arrays`](examples/24_arrays.rs) | Postgres array columns — `Vec<T>` as `TEXT[]`/`INTEGER[]`/`BIGINT[]`/`UUID[]`, an empty array against a NULL one, and `sql!{}` for the operators |

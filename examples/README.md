# qbrs examples

Runnable, end-to-end examples demonstrating qbrs against a real Postgres.
Each one is a standalone `cargo run` target under [`examples/`](examples).

## Setup

These examples (and the `qbrs-sqlx` integration test) need a real Postgres.

**Docker** (recommended for CI / sandboxed environments):

```sh
cd examples
docker compose up -d
cd -
cargo run -p qbrs-examples --example 01_select_basic
```

**No Docker** — omit `DATABASE_URL` and the examples (and
`cargo test -p qbrs-sqlx`) will download and run a real, cached, native
Postgres binary via [`postgresql_embedded`](https://docs.rs/postgresql_embedded)
on first use. (Needs normal outbound internet access — some sandboxed
environments block the binary CDN specifically even when Docker Hub is
reachable; if it times out, use the Docker path above instead.)

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
| [`10_prepared`](examples/10_prepared.rs) | `prepare!{}` — a query rendered once, reused across many typed `.execute(params)` calls |
| [`11_upsert`](examples/11_upsert.rs) | `ON CONFLICT (..) DO NOTHING` / `DO UPDATE SET ..`, gated to dialects with `SupportsOnConflict` |
| [`12_union`](examples/12_union.rs) | `UNION ALL`/`INTERSECT` across `SELECT`s with unrelated `Scope`s |
| [`13_window`](examples/13_window.rs) | `row_number()`/`rank()` `.over(window()..)` |
| [`14_cte`](examples/14_cte.rs) | `with!{}` + `cte::with(..)` — a CTE used as a real table |

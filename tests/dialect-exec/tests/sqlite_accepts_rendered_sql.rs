//! Every statement qbrs renders for `Sqlite` is handed to a real SQLite and
//! executed. String assertions elsewhere say the SQL looks right; only this
//! says SQLite agrees.

use qbrs::cte::with;
use qbrs::expr::{Value, count, sum};
use qbrs::prelude::*;
use qbrs::sql;
use sqlx::{Row, SqlitePool};

#[derive(Table)]
#[table(name = "users")]
#[allow(dead_code)]
struct Users {
    #[column(primary_key, generated)]
    id: i64,
    email: String,
    display_name: Option<String>,
}

#[derive(Table)]
#[table(name = "orders")]
#[allow(dead_code)]
struct Orders {
    #[column(primary_key, generated)]
    id: i64,
    user_id: i64,
    total: i64,
}

with! {
    struct big_orders { user_id: BigInt, total: Nullable<BigInt> }
}

async fn schema() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("open in-memory sqlite");
    for ddl in [
        "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, email TEXT NOT NULL UNIQUE, display_name TEXT)",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, total INTEGER NOT NULL)",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(ddl))
            .execute(&pool)
            .await
            .expect("create schema");
    }
    pool
}

async fn run(
    pool: &SqlitePool,
    (sql, params): (String, Vec<Value>),
) -> Vec<sqlx::sqlite::SqliteRow> {
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.clone()));
    for p in params {
        q = match p {
            Value::I32(v) => q.bind(v),
            Value::I64(v) => q.bind(v),
            Value::F64(v) => q.bind(v),
            Value::Text(v) => q.bind(v),
            Value::Bool(v) => q.bind(v),
            Value::Bytes(v) => q.bind(v),
            Value::NullI32 => q.bind(None::<i32>),
            Value::NullI64 => q.bind(None::<i64>),
            Value::NullF64 => q.bind(None::<f64>),
            Value::NullText => q.bind(None::<String>),
            Value::NullBool => q.bind(None::<bool>),
            Value::NullBytes => q.bind(None::<Vec<u8>>),
            Value::Placeholder(name) => panic!("unresolved placeholder {name}"),
            // The feature-gated column types decode through their own
            // crates, which this SQLite battery doesn't pull in — and which
            // aren't there at all unless those features are on.
            #[allow(unreachable_patterns)]
            other => panic!("no SQLite binding for {other:?}"),
        };
    }
    q.fetch_all(pool)
        .await
        .unwrap_or_else(|e| panic!("sqlite rejected `{sql}`: {e}"))
}

#[tokio::test]
async fn sqlite_executes_every_rendered_statement_shape() {
    let pool = schema().await;

    run(
        &pool,
        insert::<Sqlite, _>(users::Table)
            .values(
                UsersInsert::builder()
                    .email("ada@example.com")
                    .display_name("Ada")
                    .build(),
            )
            .values(UsersInsert::builder().email("dan@example.com").build())
            .to_sql(),
    )
    .await;

    let ids = run(
        &pool,
        insert::<Sqlite, _>(orders::Table)
            .values(OrdersInsert::builder().user_id(1).total(100).build())
            .values(OrdersInsert::builder().user_id(1).total(2000).build())
            .values(OrdersInsert::builder().user_id(2).total(30).build())
            .returning(orders::id)
            .to_sql(),
    )
    .await;
    assert_eq!(ids.len(), 3);

    run(
        &pool,
        insert::<Sqlite, _>(users::Table)
            .values(
                UsersInsert::builder()
                    .email("ada@example.com")
                    .display_name("Ada L.")
                    .build(),
            )
            .on_conflict_do_update(
                users::email,
                Assignments::from_row(UsersUpdate {
                    display_name: Some(Some("Ada Lovelace".into())),
                    ..Default::default()
                })
                .expect("display_name is set"),
            )
            .to_sql(),
    )
    .await;

    run(
        &pool,
        update::<Sqlite, _>(users::Table)
            .set(
                Assignments::from_row(UsersUpdate {
                    display_name: Some(None),
                    ..Default::default()
                })
                .expect("display_name is set"),
            )
            .filter(users::email.eq("dan@example.com"))
            .to_sql(),
    )
    .await;

    let joined = run(
        &pool,
        select((users::email, orders::total))
            .from::<Sqlite, _>(users::Table)
            .left_join(orders::Table, orders::user_id.eq(users::id))
            .filter(users::email.eq("ada@example.com"))
            .order_by(orders::total.desc())
            .limit(10)
            .to_sql(),
    )
    .await;
    assert_eq!(joined.len(), 2);

    let grouped = run(
        &pool,
        select((orders::user_id, sum(orders::total), count()))
            .from::<Sqlite, _>(orders::Table)
            .group_by(orders::user_id)
            .having(sum(orders::total).gt(50i64))
            .to_sql(),
    )
    .await;
    // Only user 1 clears the HAVING threshold.
    assert_eq!(grouped.len(), 1);

    // A bare OFFSET: SQLite has no `OFFSET` without a `LIMIT`, so the
    // dialect fills one in.
    let offset_only = run(
        &pool,
        select(users::email)
            .from::<Sqlite, _>(users::Table)
            .order_by(users::id.asc())
            .offset(1)
            .to_sql(),
    )
    .await;
    assert_eq!(offset_only.len(), 1);

    // Branches that carry their own `ORDER BY`/`LIMIT`, or a `WITH`: each
    // binds to the branch, not to the compound, which is what the derived
    // table around every SQLite branch is for.
    let paged_union = run(
        &pool,
        select((users::email,))
            .from::<Sqlite, _>(users::Table)
            .order_by(users::id.asc())
            .limit(1)
            .union(&select((users::email,)).from::<Sqlite, _>(users::Table))
            .to_sql(),
    )
    .await;
    assert_eq!(paged_union.len(), 2);

    let ordered_union = run(
        &pool,
        select((users::email,))
            .from::<Sqlite, _>(users::Table)
            .order_by(users::id.asc())
            .union(&select((users::email,)).from::<Sqlite, _>(users::Table))
            .to_sql(),
    )
    .await;
    assert_eq!(ordered_union.len(), 2);

    let cte_branch = run(
        &pool,
        select((big_orders::user_id,))
            .from::<Sqlite, _>(users::Table)
            .inner_join(
                with(
                    big_orders::Table,
                    &select((orders::user_id, sum(orders::total)))
                        .from::<Sqlite, _>(orders::Table)
                        .group_by(orders::user_id),
                ),
                big_orders::user_id.eq(users::id),
            )
            .union(&select((orders::user_id,)).from::<Sqlite, _>(orders::Table))
            .to_sql(),
    )
    .await;
    assert!(!cte_branch.is_empty());

    let union = run(
        &pool,
        select((users::email,))
            .from::<Sqlite, _>(users::Table)
            .filter(users::email.eq("ada@example.com"))
            .union(
                &select((users::email,))
                    .from::<Sqlite, _>(users::Table)
                    .filter(users::email.eq("dan@example.com")),
            )
            .to_sql(),
    )
    .await;
    assert_eq!(union.len(), 2);

    let totals = select((orders::user_id, sum(orders::total)))
        .from::<Sqlite, _>(orders::Table)
        .group_by(orders::user_id);
    let cte = run(
        &pool,
        select((users::email, big_orders::total))
            .from::<Sqlite, _>(users::Table)
            .inner_join(
                with(big_orders::Table, &totals),
                big_orders::user_id.eq(users::id),
            )
            .to_sql(),
    )
    .await;
    assert_eq!(cte.len(), 2);

    let exists_q = select(users::email).from::<Sqlite, _>(users::Table);
    let correlated = exists_q
        .correlated(orders::Table, orders::id)
        .filter(orders::user_id.eq(users::id))
        .exists();
    let rows = run(&pool, exists_q.filter(correlated).to_sql()).await;
    assert_eq!(rows.len(), 2);

    qbrs::label!(rank_in_user);

    let ranked = run(
        &pool,
        select((
            orders::user_id,
            row_number()
                .over(
                    window()
                        .partition_by(orders::user_id)
                        .order_by(orders::total.desc()),
                )
                .label(label::rank_in_user),
        ))
        .from::<Sqlite, _>(orders::Table)
        .to_sql(),
    )
    .await;
    assert_eq!(ranked.len(), 3);

    let right = run(
        &pool,
        select((users::email, orders::total))
            .from::<Sqlite, _>(orders::Table)
            .right_join(users::Table, orders::user_id.eq(users::id))
            .to_sql(),
    )
    .await;
    assert_eq!(right.len(), 3);

    let raw = run(
        &pool,
        select((users::email,))
            .from::<Sqlite, _>(users::Table)
            .filter(sql!(Bool, "length(email) > ?", 3i64))
            .to_sql(),
    )
    .await;
    assert_eq!(raw.len(), 2);

    let distinct = run(
        &pool,
        select((users::email,))
            .from::<Sqlite, _>(users::Table)
            .inner_join(orders::Table, orders::user_id.eq(users::id))
            .distinct()
            .to_sql(),
    )
    .await;
    assert_eq!(distinct.len(), 2);

    let bulk = run(
        &pool,
        insert::<Sqlite, _>(orders::Table)
            .values_all((10..13).map(|n| OrdersInsert::builder().user_id(1).total(n).build()))
            .expect("three rows")
            .returning(orders::id)
            .to_sql(),
    )
    .await;
    assert_eq!(bulk.len(), 3);

    let deleted = run(
        &pool,
        delete::<Sqlite, _>(orders::Table)
            .filter(orders::total.lt(50i64))
            .returning(orders::id)
            .to_sql(),
    )
    .await;
    assert_eq!(deleted.len(), 4);

    let remaining = run(
        &pool,
        select(count()).from::<Sqlite, _>(orders::Table).to_sql(),
    )
    .await;
    assert_eq!(remaining[0].get::<i64, _>(0), 2);

    // `count_sql` is its own rendering — the query wrapped in a total, with
    // its paging dropped — so it is executed here rather than only asserted
    // as a string.
    let total = run(
        &pool,
        select((orders::id,))
            .from::<Sqlite, _>(orders::Table)
            .filter(orders::total.gt(0i64))
            .order_by(orders::id.desc())
            .limit(1)
            .count_sql(),
    )
    .await;
    assert_eq!(total[0].get::<i64, _>(0), 2);

    let grouped_total = run(
        &pool,
        select((orders::user_id, count()))
            .from::<Sqlite, _>(orders::Table)
            .group_by(orders::user_id)
            .having_all(vec![predicate(count().gte(1i64))])
            .count_sql(),
    )
    .await;
    assert_eq!(grouped_total[0].get::<i64, _>(0), 1);

    let full = run(
        &pool,
        select((users::email, orders::total))
            .from::<Sqlite, _>(users::Table)
            .full_join(orders::Table, orders::user_id.eq(users::id))
            .to_sql(),
    )
    .await;
    assert!(!full.is_empty());

    let intersected = run(
        &pool,
        select((users::email,))
            .from::<Sqlite, _>(users::Table)
            .intersect(
                &select((users::email,))
                    .from::<Sqlite, _>(users::Table)
                    .filter(users::email.like("ada%")),
            )
            .to_sql(),
    )
    .await;
    assert_eq!(intersected.len(), 1);

    let excepted = run(
        &pool,
        select((users::email,))
            .from::<Sqlite, _>(users::Table)
            .except(
                &select((users::email,))
                    .from::<Sqlite, _>(users::Table)
                    .filter(users::email.like("ada%")),
            )
            .to_sql(),
    )
    .await;
    assert_eq!(excepted.len(), 1);

    let erased = run(
        &pool,
        select((users::email,))
            .from::<Sqlite, _>(users::Table)
            .erase()
            .limit(1)
            .to_sql(),
    )
    .await;
    assert_eq!(erased.len(), 1);

    let in_list = run(
        &pool,
        select((users::email,))
            .from::<Sqlite, _>(users::Table)
            .filter(users::email.is_in(["ada@example.com".to_string()]))
            .filter(users::id.is_in(Vec::<i64>::new()))
            .to_sql(),
    )
    .await;
    assert!(in_list.is_empty());
}

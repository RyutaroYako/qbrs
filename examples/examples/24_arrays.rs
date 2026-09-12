//! Postgres array columns: `TEXT[]`, `INTEGER[]` and `BIGINT[]` declared in
//! a schema as `Vec<T>`, bound as one parameter and decoded back. `UUID[]`
//! is the same shape as a `Vec<Uuid>` behind the `uuid` feature.
//! `Vec<u8>` stays `bytea` — the element type is what decides.
//! `.eq_any(..)` asks whether a value is one of an array column's elements
//! (`x = ANY(arr)`).
//! Known limitation: the array *operators* (`@>`, `&&`, `array_append`) are
//! not built; those go through `sql!{}`, as the last query shows.
//! Run: `cargo run -p qbrs-examples --example 24_arrays`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[derive(Table)]
#[table(name = "mailing_lists")]
#[allow(dead_code)]
struct MailingLists {
    #[column(primary_key, generated)]
    id: i64,
    name: String,
    recipients: Vec<String>,
    retry_delays: Vec<i32>,
    sent_message_ids: Vec<i64>,
    cc: Option<Vec<String>>,
}

/// A plain domain struct, filled by field name like any other row.
#[derive(qbrs::FromRow, Debug)]
struct List {
    name: String,
    recipients: Vec<String>,
    cc: Option<Vec<String>>,
}

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;

    // `setup_db()` resets the shared schema; a table an example makes is
    // its own to reset, or a second run against a `DATABASE_URL` server
    // finds it already there.
    sqlx::query("DROP TABLE IF EXISTS mailing_lists")
        .execute(&pool)
        .await
        .expect("drop mailing_lists");
    sqlx::query(
        "CREATE TABLE mailing_lists (
            id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            name TEXT NOT NULL,
            recipients TEXT[] NOT NULL,
            retry_delays INTEGER[] NOT NULL,
            sent_message_ids BIGINT[] NOT NULL,
            cc TEXT[]
        )",
    )
    .execute(&pool)
    .await
    .expect("create mailing_lists");

    insert(mailing_lists::Table)
        .values(
            MailingListsInsert::builder()
                .name("ops")
                .recipients(vec![
                    "ops@example.com".to_string(),
                    "sre@example.com".to_string(),
                ])
                .retry_delays(vec![1, 5, 30])
                .sent_message_ids(vec![9_000_000_000i64, 9_000_000_001])
                .cc(Some(vec!["cto@example.com".to_string()]))
                .build(),
        )
        // An empty array is a value; an omitted nullable one is a NULL of
        // that array's own type, and the two stay apart.
        .values(
            MailingListsInsert::builder()
                .name("quiet")
                .recipients(Vec::new())
                .retry_delays(Vec::new())
                .sent_message_ids(Vec::new())
                .build(),
        )
        .execute(&pool)
        .await
        .expect("seed the lists");

    let lists: Vec<List> = select(mailing_lists::All)
        .from(mailing_lists::Table)
        .order_by(mailing_lists::id.asc())
        .load(&pool)
        .await
        .expect("read the lists back")
        .into_structs();
    for list in &lists {
        println!("{}: {:?} (cc {:?})", list.name, list.recipients, list.cc);
    }
    assert_eq!(lists[1].recipients, Vec::<String>::new());
    assert_eq!(lists[1].cc, None);

    // An array compares as a whole — one bind parameter against one column,
    // not a rendered list.
    let exact: i64 = select(qbrs::expr::count())
        .from(mailing_lists::Table)
        .filter(mailing_lists::retry_delays.eq(vec![1, 5, 30]))
        .load_one(&pool)
        .await
        .expect("compare a whole array")
        .expect("one row");
    println!("lists whose retry schedule is exactly [1, 5, 30]: {exact}");
    assert_eq!(exact, 1);

    // `BIGINT[]` is the same shape one element type over.
    let sent: Vec<Vec<i64>> = select(mailing_lists::sent_message_ids)
        .from(mailing_lists::Table)
        .order_by(mailing_lists::id.asc())
        .load(&pool)
        .await
        .expect("read the bigint array");
    println!("message ids: {sent:?}");
    assert_eq!(sent, vec![vec![9_000_000_000i64, 9_000_000_001], vec![]]);

    // "Is this value one of the elements" is the question `is_in` asks of
    // a written-out list, asked of an array the database unnests — so the
    // array can be a column, which a list cannot.
    let listing: Vec<String> = select(mailing_lists::name)
        .from(mailing_lists::Table)
        .filter(
            "sre@example.com"
                .to_string()
                .eq_any(mailing_lists::recipients),
        )
        .load(&pool)
        .await
        .expect("membership in an array column");
    println!("lists with sre@example.com as a recipient: {listing:?}");
    assert_eq!(listing, vec!["ops".to_string()]);

    // Asking whether an array *contains* another is an operator, and those
    // aren't built — the escape hatch takes the column and the value as
    // slots, so both are still checked and bound.
    let containing: Vec<String> = select(mailing_lists::name)
        .from(mailing_lists::Table)
        .filter(qbrs::sql!(
            Bool,
            "? @> ARRAY[?]::text[]",
            mailing_lists::recipients,
            "sre@example.com"
        ))
        .load(&pool)
        .await
        .expect("containment through the escape hatch");
    println!("lists containing sre@example.com: {containing:?}");
    assert_eq!(containing, vec!["ops".to_string()]);
}

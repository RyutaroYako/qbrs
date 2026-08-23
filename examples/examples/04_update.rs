//! UPDATE: only the fields a request actually set are sent, and a nullable
//! column tells "leave alone" apart from "set to NULL" — the builder's
//! `.column(value)` vs `.column_null()`.
//! Run: `cargo run -p qbrs-examples --example 04_update`

use qbrs::prelude::*;
use qbrs_examples::*;
use qbrs_sqlx::prelude::*;

#[tokio::main]
async fn main() {
    let (pool, _db) = setup_db().await;
    seed(&pool).await;

    let ada_id: i64 = select(users::id)
        .from(users::Table)
        .filter(users::email.eq("ada@example.com"))
        .load_one(&pool)
        .await
        .expect("query ada")
        .expect("ada exists");

    // A request's fields, mapped across one for one: `email` was not sent,
    // so it is absent from the rendered SQL — not set to NULL, and not
    // left unchanged via a redundant `email = email` self-assignment.
    let requested_email: Option<String> = None;
    let requested_name: Option<String> = Some("Ada, Countess of Lovelace".into());
    let affected = update(users::Table)
        .set(
            Assignments::from_row(
                UsersUpdate::builder()
                    .email(requested_email)
                    .display_name(requested_name)
                    .build(),
            )
            .expect("display_name is set"),
        )
        .filter(users::id.eq(ada_id))
        .execute(&pool)
        .await
        .expect("update display_name");
    println!("renamed {affected} row(s)");

    // An assignment the database computes: the value never passes through
    // this process, so `now()` is the database's clock and a counter can be
    // bumped without reading it first.
    let stamped = update(users::Table)
        .set(
            Assignments::from_row(UsersUpdate {
                display_name: Some(Some("Ada L.".into())),
                ..Default::default()
            })
            .expect("display_name is set"),
        )
        .set_to(users::email, sql!(Text, "lower(?)", users::email))
        .filter(users::id.eq(ada_id))
        .execute(&pool)
        .await
        .expect("stamp");
    println!("normalised {stamped} row(s)");

    // Clearing the column is its own call, distinct from having nothing to
    // say about it — the struct literal spells the same two states
    // `Some(None)` and `None`.
    let cleared = update(users::Table)
        .set(
            Assignments::from_row(UsersUpdate::builder().display_name_null().build())
                .expect("display_name is set"),
        )
        .filter(users::id.eq(ada_id))
        .returning(users::display_name)
        .load(&pool)
        .await
        .expect("clear display_name");
    println!("after clearing: {cleared:?}");
    assert_eq!(cleared, vec![None]);

    // The same NULL as an assignment rather than a request field, which is
    // where an `Option` has no `None` to be. Assigning a column twice keeps
    // the last assignment, so this one wins over the request's.
    let relabelled = update(users::Table)
        .set(
            Assignments::from_row(UsersUpdate {
                display_name: Some(Some("Overwritten below".into())),
                ..Default::default()
            })
            .expect("display_name is set"),
        )
        .set_to(users::display_name, null::<Text>())
        .filter(users::id.eq(ada_id))
        .returning(users::display_name)
        .load(&pool)
        .await
        .expect("null out display_name");
    assert_eq!(relabelled, vec![None]);
    println!("still NULL after a layered assignment: {relabelled:?}");
}

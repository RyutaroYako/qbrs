//! UPDATE: only the fields a request actually set are sent, and a nullable
//! column tells "leave alone" apart from "set to NULL". That is the
//! builder's `.column(value)` vs `.column_null()`.
//!
//! A request field that was not sent is absent from the rendered SQL. It is
//! neither set to NULL nor rewritten as a redundant `email = email`.
//! `.set_to(..)` assigns a value the database computes, which never passes
//! through this process. The struct literal spells "clear it" and "leave it
//! alone" as `Some(None)` and `None`. `null::<T>()` is that NULL as an
//! assignment, where an `Option` has no `None` to be. A column assigned twice
//! keeps the last assignment.
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

    let email_not_in_request: Option<String> = None;
    let requested_name: Option<String> = Some("Ada, Countess of Lovelace".into());
    let affected = update(users::Table)
        .set(
            Assignments::from_row(
                UsersUpdate::builder()
                    .email(email_not_in_request)
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

    let lowercased_in_database = update(users::Table)
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
        .expect("lowercase email in the database");
    println!("normalised {lowercased_in_database} row(s)");

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
    assert_eq!(
        relabelled,
        vec![None],
        "the later NULL assignment wins over the request's value"
    );
    println!("still NULL after a layered assignment: {relabelled:?}");
}

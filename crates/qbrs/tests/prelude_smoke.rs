use qbrs::prelude::*;

#[derive(Table)]
#[table(name = "users")]
#[allow(dead_code)]
struct Users {
    #[column(primary_key)]
    id: i64,
    tags: Vec<String>,
}

#[test]
fn the_prelude_leaves_sort_on_a_vec_of_plain_values_to_the_slice() {
    let mut names = vec!["dan".to_string(), "ada".to_string()];
    names.sort();
    assert_eq!(names, ["ada", "dan"]);
}

#[test]
fn the_prelude_leaves_eq_and_lt_on_plain_values_to_partial_eq_and_partial_ord() {
    let ada = "ada".to_string();
    assert!(ada.eq(&"ada".to_string()));
    assert!(1i64.lt(&2i64));
}

#[test]
fn eq_any_still_takes_a_plain_value_as_its_receiver() {
    let (sql, params) = select(users::id)
        .from(users::Table)
        .filter("rust".to_string().eq_any(users::tags))
        .to_sql(Postgres);
    assert_eq!(
        sql,
        "SELECT \"users\".\"id\" FROM \"users\" WHERE ($1 = ANY(\"users\".\"tags\"))"
    );
    assert_eq!(params.len(), 1);
}

use compile_bench::{T00, T01, T02, T03, T04, T05, T06, T07, T08, T09};
use compile_bench::{assert_contains, assert_map_nullable, assert_superset, req_of, scope_of};

type Scope = scope_of!(T00, T01, T02, T03, T04, T05, T06, T07, T08, T09);

fn main() {
    assert_contains::<Scope, T00, _>(); // worst case: last in the list, deepest recursion
    assert_contains::<Scope, T09, _>(); // best case: first in the list (Here)
    assert_superset::<Scope, req_of!(T01, T08), _>();
    assert_map_nullable::<Scope>();
}

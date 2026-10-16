use compile_bench::{T00, T01, T02, T03, T04};
use compile_bench::{assert_contains, assert_map_nullable, assert_superset, req_of, scope_of};

type Scope = scope_of!(T00, T01, T02, T03, T04);

type FirstFromd = T00;
type LastJoined = T04;

fn main() {
    assert_contains::<Scope, FirstFromd, _>();
    assert_contains::<Scope, LastJoined, _>();
    assert_superset::<Scope, req_of!(T01, T03), _>();
    assert_map_nullable::<Scope>();
}

//! zkVM guest: checks the sealed-patch relation and commits the public statement.
use arkiv_git_zk_core::{Private, check};
use risc0_zkvm::guest::env;

fn main() {
    let private: Private = env::read();
    let spec: String = env::read();
    match check(&private, &spec) {
        Ok(public) => env::commit(&public),
        Err(e) => panic!("relation does not hold: {e:?}"),
    }
}

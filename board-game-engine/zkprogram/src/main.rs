#![no_main]

extern crate alloc;

use board_game_engine::game::GameState;
use hyle_contract_sdk::guest::execute;
use hyle_contract_sdk::guest::GuestEnv;
use hyle_contract_sdk::guest::SP1Env;

sp1_zkvm::entrypoint!(main);

fn main() {
    let env = SP1Env {};
    let input = env.read();
    let (_, output) = execute::<GameState>(&input);
    env.commit(&output);
}

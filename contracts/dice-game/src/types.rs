use soroban_sdk::{contracttype, Address};

#[derive(Clone)]
#[contracttype]
pub struct Config {
    pub admin: Address,
    pub token: Address,
    pub entry_fee: i128,
    pub payout_multiplier_bps: u32,
    pub max_rolls: u32,
    pub round_seconds: u64,
}

/// The entire game is a pure deterministic function of (seed, the sequence
/// of hold_masks submitted to claim), so there is nothing "live" to track
/// between start_game and claim - no dice, no roll_count, no status. A
/// session existing at all means "Playing" (a win clears it via claim; a
/// loss is, by design, never written back - see claim()'s doc comment in
/// lib.rs). start_game's overwrite guard is a pure now >= deadline_ts check.
#[derive(Clone, Debug, PartialEq)]
#[contracttype]
pub struct Session {
    pub seed: u64,
    pub max_rolls: u32,
    pub deadline_ts: u64,
    pub entry_fee: i128,
    pub payout_multiplier_bps: u32,
}

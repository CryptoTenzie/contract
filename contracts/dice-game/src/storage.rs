use soroban_sdk::{contracttype, Address, Env};

use crate::errors::Error;
use crate::types::{Config, Session};

// Ledgers close roughly every 5s on testnet, so ~17280 ledgers is ~1 day.
// Bump TTL well before expiry so a session/config entry never gets archived
// out from under an in-progress game.
const LIFETIME_THRESHOLD: u32 = 17280;
const BUMP_AMOUNT: u32 = 34560;

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Config,
    Session(Address),
}

pub fn config_exists(env: &Env) -> bool {
    env.storage().instance().has(&DataKey::Config)
}

pub fn load_config(env: &Env) -> Result<Config, Error> {
    env.storage()
        .instance()
        .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(Error::NotInitialized)
}

pub fn save_config(env: &Env, config: &Config) {
    env.storage().instance().set(&DataKey::Config, config);
    env.storage()
        .instance()
        .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
}

pub fn load_session(env: &Env, player: &Address) -> Option<Session> {
    let key = DataKey::Session(player.clone());
    let session = env.storage().persistent().get(&key);
    if session.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
    }
    session
}

pub fn save_session(env: &Env, player: &Address, session: &Session) {
    let key = DataKey::Session(player.clone());
    env.storage().persistent().set(&key, session);
    env.storage()
        .persistent()
        .extend_ttl(&key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
}

pub fn clear_session(env: &Env, player: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::Session(player.clone()));
}

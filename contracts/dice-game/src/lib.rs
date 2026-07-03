#![no_std]

mod dice;
mod errors;
mod storage;
mod types;

use soroban_sdk::{contract, contractimpl, token, Address, Env, Vec};

pub use crate::errors::Error;
pub use crate::types::{Config, Session};

use crate::storage::{clear_session, config_exists, load_config, load_session, save_config, save_session};

#[contract]
pub struct DiceGame;

#[contractimpl]
impl DiceGame {
    /// One-time setup. Must be called once, right after deploy, by the
    /// intended admin (who must sign/authorize this call).
    pub fn initialize(
        env: Env,
        admin: Address,
        token: Address,
        entry_fee: i128,
        payout_multiplier_bps: u32,
        max_rolls: u32,
        round_seconds: u64,
    ) -> Result<(), Error> {
        if config_exists(&env) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        let config = Config {
            admin,
            token,
            entry_fee,
            payout_multiplier_bps,
            max_rolls,
            round_seconds,
        };
        save_config(&env, &config);
        Ok(())
    }

    /// Player pays the entry fee in XLM and the contract commits to a single
    /// random seed on-chain. The entire game is a deterministic function of
    /// (seed, the sequence of hold_masks the player later submits to
    /// claim()) - dice rolling itself never touches the chain, so the
    /// player can play instantly in the browser with zero blockchain calls
    /// per roll (see dice.rs). Only one active session per player at a
    /// time; a prior session can be overwritten once its deadline passes.
    pub fn start_game(env: Env, player: Address) -> Result<Session, Error> {
        player.require_auth();
        let config = load_config(&env)?;

        if let Some(existing) = load_session(&env, &player) {
            if env.ledger().timestamp() < existing.deadline_ts {
                return Err(Error::SessionActive);
            }
        }

        let token_client = token::Client::new(&env, &config.token);
        token_client.transfer(&player, &env.current_contract_address(), &config.entry_fee);

        let seed: u64 = env.prng().gen();
        let session = Session {
            seed,
            max_rolls: config.max_rolls,
            deadline_ts: env.ledger().timestamp() + config.round_seconds,
            entry_fee: config.entry_fee,
            payout_multiplier_bps: config.payout_multiplier_bps,
        };
        save_session(&env, &player, &session);
        Ok(session)
    }

    /// Verifies a win and pays out. hold_mask_sequence is the player's full
    /// history of hold choices, one entry per roll they performed locally -
    /// array position IS the roll_index (no explicit index is ever
    /// accepted), which is what makes reordering or skipping a roll
    /// structurally impossible rather than merely inconvenient. The replay
    /// - not any separately-stored state - IS the game: starting from
    /// dice::compute_dice(seed, 0), each mask in the sequence is applied via
    /// dice::reroll_unheld_deterministic in order, and the result must be
    /// all-equal only once the sequence is fully consumed (an early win
    /// with unconsumed trailing entries is rejected - those entries can't
    /// be used to cheat since held dice never change, but rejecting them
    /// keeps roll_count meaningful and bounds replay cost to exactly what
    /// was needed).
    ///
    /// A losing game costs no transaction at all: the player simply never
    /// calls claim, the entry fee is already forfeit, and the stale session
    /// becomes overwritable by their next start_game once deadline_ts
    /// passes.
    ///
    /// Fairness note: since the whole game is "does a winning sequence
    /// exist for this seed within max_rolls", a player can explore a fixed
    /// seed's decision tree client-side before deciding what to submit,
    /// rather than committing to each choice turn-by-turn as in the
    /// previous per-roll-transaction design. This can only help them find a
    /// winning path that already exists for that seed - it can't change
    /// whether one exists - so it doesn't move the underlying odds. This is
    /// a deliberate, documented trade-off, not an oversight.
    pub fn claim(env: Env, player: Address, hold_mask_sequence: Vec<u32>) -> Result<i128, Error> {
        player.require_auth();
        let session = load_session(&env, &player).ok_or(Error::NoActiveSession)?;

        if env.ledger().timestamp() >= session.deadline_ts {
            return Err(Error::SessionExpired);
        }
        if hold_mask_sequence.is_empty() {
            return Err(Error::InvalidRollSequence);
        }
        if hold_mask_sequence.len() > session.max_rolls {
            return Err(Error::MaxRollsReached);
        }

        let mut dice = dice::compute_dice(&env, session.seed, 0);
        let roll_count = hold_mask_sequence.len();
        for (i, hold_mask) in hold_mask_sequence.iter().enumerate() {
            if hold_mask >= (1 << 10) {
                return Err(Error::InvalidHoldMask);
            }
            let roll_index = (i + 1) as u32;
            dice = dice::reroll_unheld_deterministic(&env, &dice, hold_mask, session.seed, roll_index);

            let is_last = roll_index == roll_count;
            if dice::all_equal(&dice) && !is_last {
                return Err(Error::InvalidRollSequence);
            }
        }
        if !dice::all_equal(&dice) {
            return Err(Error::NotWon);
        }

        let payout = session
            .entry_fee
            .checked_mul(session.payout_multiplier_bps as i128)
            .and_then(|v| v.checked_div(100))
            .ok_or(Error::Overflow)?;

        let config = load_config(&env)?;
        let token_client = token::Client::new(&env, &config.token);
        let reserve = token_client.balance(&env.current_contract_address());
        if reserve < payout {
            return Err(Error::InsufficientReserve);
        }

        // Clear the session before the transfer (checks-effects-interactions),
        // even though the native XLM SAC has no reentrant callback hooks.
        clear_session(&env, &player);
        token_client.transfer(&env.current_contract_address(), &player, &payout);
        Ok(payout)
    }

    /// Public read of a player's session. Not gated by auth - once dice are
    /// on-chain they aren't a secret, so any address's session state can be
    /// queried by anyone. This is a deliberate design choice, not an oversight.
    pub fn get_session(env: Env, player: Address) -> Option<Session> {
        load_session(&env, &player)
    }

    /// Public read of the current game config, e.g. so the frontend can show
    /// the entry fee before a player has an active session.
    pub fn get_config(env: Env) -> Result<Config, Error> {
        load_config(&env)
    }

    /// Admin-only tuning of game parameters. Live sessions are unaffected
    /// since entry_fee/payout_multiplier are snapshotted into each Session
    /// at start_game time.
    pub fn set_config(
        env: Env,
        entry_fee: i128,
        payout_multiplier_bps: u32,
        max_rolls: u32,
        round_seconds: u64,
    ) -> Result<(), Error> {
        let mut config = load_config(&env)?;
        config.admin.require_auth();

        config.entry_fee = entry_fee;
        config.payout_multiplier_bps = payout_multiplier_bps;
        config.max_rolls = max_rolls;
        config.round_seconds = round_seconds;
        save_config(&env, &config);
        Ok(())
    }

    /// Open to anyone - lets the house reserve be topped up so the contract
    /// can always cover payouts above 1x the collected entry fees.
    pub fn deposit(env: Env, from: Address, amount: i128) -> Result<(), Error> {
        from.require_auth();
        let config = load_config(&env)?;
        let token_client = token::Client::new(&env, &config.token);
        token_client.transfer(&from, &env.current_contract_address(), &amount);
        Ok(())
    }

    /// Admin-only withdrawal of house reserve.
    pub fn withdraw(env: Env, amount: i128) -> Result<(), Error> {
        let config = load_config(&env)?;
        config.admin.require_auth();
        let token_client = token::Client::new(&env, &config.token);
        token_client.transfer(&env.current_contract_address(), &config.admin, &amount);
        Ok(())
    }
}

mod test;

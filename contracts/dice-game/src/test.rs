#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::token::{StellarAssetClient, TokenClient};

fn create_token<'a>(env: &Env, admin: &Address) -> (TokenClient<'a>, StellarAssetClient<'a>) {
    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    (
        TokenClient::new(env, &sac.address()),
        StellarAssetClient::new(env, &sac.address()),
    )
}

const ENTRY_FEE: i128 = 10_000_000; // 1 XLM
const PAYOUT_MULTIPLIER_BPS: u32 = 200; // 2x
const MAX_ROLLS: u32 = 15;
const ROUND_SECONDS: u64 = 180;
const HOLD_ALL: u32 = 0b11_1111_1111;

struct Setup {
    env: Env,
    contract_id: Address,
    admin: Address,
    token: TokenClient<'static>,
    token_admin: StellarAssetClient<'static>,
    player: Address,
}

fn setup() -> Setup {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let player = Address::generate(&env);
    let (token, token_admin) = create_token(&env, &admin);

    let contract_id = env.register(DiceGame, ());
    let client = DiceGameClient::new(&env, &contract_id);
    client.initialize(
        &admin,
        &token.address,
        &ENTRY_FEE,
        &PAYOUT_MULTIPLIER_BPS,
        &MAX_ROLLS,
        &ROUND_SECONDS,
    );

    // Fund the player and pre-fund the contract's house reserve so payouts
    // above 1x entry fee can always be covered in tests.
    token_admin.mint(&player, &(ENTRY_FEE * 10));
    token_admin.mint(&contract_id, &(ENTRY_FEE * 10));

    Setup {
        env,
        contract_id,
        admin,
        token,
        token_admin,
        player,
    }
}

fn new_funded_player(s: &Setup) -> Address {
    let player = Address::generate(&s.env);
    s.token_admin.mint(&player, &(ENTRY_FEE * 10));
    player
}

/// Mirrors the classic "hold your most common current face, reroll the
/// rest" Tenzies strategy - not the game's actual client (that lives in
/// diceRng.js), just a reference strategy for exercising the real replay
/// primitives (dice::compute_dice / reroll_unheld_deterministic) the same
/// way claim() will. Returns (final_dice, hold_mask_sequence). If the
/// dice are already all-equal (including a natural win on the initial
/// roll), the sequence gets one trailing HOLD_ALL entry, matching how a
/// real player would need to submit at least one entry to claim() even a
/// zero-reroll win.
fn play_greedy(env: &Env, seed: u64, max_rolls: u32) -> (Vec<u32>, Vec<u32>) {
    let mut dice = dice::compute_dice(env, seed, 0);
    let mut sequence: Vec<u32> = Vec::new(env);

    if dice::all_equal(&dice) {
        // Natural win on the initial deal, before any rerolls at all - the
        // player still needs one entry to claim() (which requires a
        // non-empty sequence), and this is the ONLY case that needs a
        // synthetic hold-everything entry. Once at least one real reroll
        // has happened, that reroll's own mask is always the correct final
        // entry - appending anything after it would make claim() see it as
        // an early win with trailing entries and correctly reject it.
        sequence.push_back(HOLD_ALL);
        return (dice, sequence);
    }

    let mut roll_index = 0u32;
    while roll_index < max_rolls {
        let mut counts = [0u32; 7];
        for i in 0..dice.len() {
            counts[dice.get(i).unwrap() as usize] += 1;
        }
        let mut mode_face = 1u32;
        let mut mode_count = 0u32;
        for face in 1u32..=6 {
            if counts[face as usize] > mode_count {
                mode_count = counts[face as usize];
                mode_face = face;
            }
        }
        let mut mask = 0u32;
        for i in 0..dice.len() {
            if dice.get(i).unwrap() == mode_face {
                mask |= 1 << i;
            }
        }

        roll_index += 1;
        dice = dice::reroll_unheld_deterministic(env, &dice, mask, seed, roll_index);
        sequence.push_back(mask);

        if dice::all_equal(&dice) {
            break;
        }
    }
    (dice, sequence)
}

/// Retries fresh players/seeds until the greedy strategy wins within
/// max_rolls, up to `attempts` tries. PRNG isn't seeded deterministically
/// in tests, and not every seed is winnable within max_rolls even with
/// optimal play (that's the intended difficulty), so we search rather than
/// assume the first seed converges.
fn find_winning_sequence(s: &Setup, attempts: u32) -> Option<(Address, u64, Vec<u32>)> {
    let client = DiceGameClient::new(&s.env, &s.contract_id);
    for _ in 0..attempts {
        let player = new_funded_player(s);
        let session = client.start_game(&player);
        let (final_dice, sequence) = play_greedy(&s.env, session.seed, session.max_rolls);
        if dice::all_equal(&final_dice) {
            return Some((player, session.seed, sequence));
        }
    }
    None
}

#[test]
fn test_win_and_claim_pays_out_and_clears_session() {
    let s = setup();
    let client = DiceGameClient::new(&s.env, &s.contract_id);

    let (player, _seed, sequence) =
        find_winning_sequence(&s, 30).expect("greedy strategy should win within 30 fresh seeds");

    let balance_before = s.token.balance(&player);
    let payout = client.claim(&player, &sequence);
    assert_eq!(payout, ENTRY_FEE * PAYOUT_MULTIPLIER_BPS as i128 / 100);
    assert_eq!(s.token.balance(&player), balance_before + payout);

    // Session cleared - a new game can be started immediately.
    let fresh = client.start_game(&player);
    assert_eq!(fresh.max_rolls, MAX_ROLLS);
}

#[test]
fn test_losing_game_costs_no_transaction_and_cannot_be_claimed() {
    let s = setup();
    let client = DiceGameClient::new(&s.env, &s.contract_id);

    // Force a near-certain loss deterministically (rather than searching
    // for an unlucky seed) by giving the player only one roll beyond the
    // initial deal - the odds of 10 dice naturally matching are negligible.
    client.set_config(&ENTRY_FEE, &PAYOUT_MULTIPLIER_BPS, &1, &ROUND_SECONDS);
    let session = client.start_game(&s.player);
    assert_eq!(session.max_rolls, 1);

    let (final_dice, sequence) = play_greedy(&s.env, session.seed, session.max_rolls);
    assert!(
        !dice::all_equal(&final_dice),
        "expected this to be a loss given max_rolls=1"
    );

    let result = client.try_claim(&s.player, &sequence);
    assert_eq!(result, Err(Ok(Error::NotWon)));

    // The forfeited entry fee isn't refunded, and no further transaction
    // was required to "record" the loss - the stale session just becomes
    // overwritable once its deadline passes.
    s.env
        .ledger()
        .set_timestamp(s.env.ledger().timestamp() + ROUND_SECONDS + 1);
    let fresh = client.start_game(&s.player);
    assert_eq!(fresh.max_rolls, 1);
}

#[test]
fn test_start_game_rejected_while_session_active() {
    let s = setup();
    let client = DiceGameClient::new(&s.env, &s.contract_id);

    client.start_game(&s.player);
    let result = client.try_start_game(&s.player);
    assert_eq!(result, Err(Ok(Error::SessionActive)));
}

#[test]
fn test_claim_after_deadline_rejected_even_with_winning_sequence() {
    let s = setup();
    let client = DiceGameClient::new(&s.env, &s.contract_id);

    let (player, _seed, sequence) =
        find_winning_sequence(&s, 30).expect("greedy strategy should win within 30 fresh seeds");

    s.env
        .ledger()
        .set_timestamp(s.env.ledger().timestamp() + ROUND_SECONDS + 1);

    let result = client.try_claim(&player, &sequence);
    assert_eq!(result, Err(Ok(Error::SessionExpired)));

    // Expired session should be overwritable by a fresh start_game.
    let session = client.start_game(&player);
    assert_eq!(session.max_rolls, MAX_ROLLS);
}

#[test]
fn test_claim_without_win_fails() {
    let s = setup();
    let client = DiceGameClient::new(&s.env, &s.contract_id);

    client.start_game(&s.player);
    // A single "reroll everything" entry is exceedingly unlikely to
    // naturally match all 10 dice.
    let mut sequence: Vec<u32> = Vec::new(&s.env);
    sequence.push_back(0);
    let result = client.try_claim(&s.player, &sequence);
    assert_eq!(result, Err(Ok(Error::NotWon)));
}

#[test]
fn test_empty_sequence_rejected() {
    let s = setup();
    let client = DiceGameClient::new(&s.env, &s.contract_id);

    client.start_game(&s.player);
    let empty: Vec<u32> = Vec::new(&s.env);
    let result = client.try_claim(&s.player, &empty);
    assert_eq!(result, Err(Ok(Error::InvalidRollSequence)));
}

#[test]
fn test_sequence_longer_than_max_rolls_rejected() {
    let s = setup();
    let client = DiceGameClient::new(&s.env, &s.contract_id);

    client.start_game(&s.player);
    let mut sequence: Vec<u32> = Vec::new(&s.env);
    for _ in 0..(MAX_ROLLS + 1) {
        sequence.push_back(0);
    }
    let result = client.try_claim(&s.player, &sequence);
    assert_eq!(result, Err(Ok(Error::MaxRollsReached)));
}

#[test]
fn test_early_win_with_trailing_entries_rejected() {
    let s = setup();
    let client = DiceGameClient::new(&s.env, &s.contract_id);

    let (player, _seed, mut sequence) =
        find_winning_sequence(&s, 30).expect("greedy strategy should win within 30 fresh seeds");
    // Append a redundant trailing entry after the winning roll.
    sequence.push_back(HOLD_ALL);

    let result = client.try_claim(&player, &sequence);
    assert_eq!(result, Err(Ok(Error::InvalidRollSequence)));
}

#[test]
fn test_set_config_updates_future_sessions_not_live_ones() {
    let s = setup();
    let client = DiceGameClient::new(&s.env, &s.contract_id);

    let live_session = client.start_game(&s.player);
    assert_eq!(live_session.entry_fee, ENTRY_FEE);

    client.set_config(&(ENTRY_FEE * 2), &(PAYOUT_MULTIPLIER_BPS * 2), &MAX_ROLLS, &ROUND_SECONDS);

    // The live session keeps its snapshotted terms.
    let still_live = client.get_session(&s.player).unwrap();
    assert_eq!(still_live.entry_fee, ENTRY_FEE);
    assert_eq!(still_live.payout_multiplier_bps, PAYOUT_MULTIPLIER_BPS);
}

#[test]
fn test_invalid_hold_mask_rejected() {
    let s = setup();
    let client = DiceGameClient::new(&s.env, &s.contract_id);

    client.start_game(&s.player);
    let mut sequence: Vec<u32> = Vec::new(&s.env);
    sequence.push_back(1u32 << 10);
    let result = client.try_claim(&s.player, &sequence);
    assert_eq!(result, Err(Ok(Error::InvalidHoldMask)));
}

#[test]
fn test_insufficient_reserve_guarded() {
    let s = setup();
    let client = DiceGameClient::new(&s.env, &s.contract_id);

    // A deliberately huge multiplier makes the computed payout vastly
    // exceed the test's funded house reserve (entry_fee * u32::MAX isn't
    // large enough to overflow i128 itself - checked_mul/checked_div both
    // succeed - so this exercises the reserve guard specifically, not the
    // overflow guard).
    client.set_config(&ENTRY_FEE, &u32::MAX, &MAX_ROLLS, &ROUND_SECONDS);

    let (player, _seed, sequence) =
        find_winning_sequence(&s, 30).expect("greedy strategy should win within 30 fresh seeds");
    let result = client.try_claim(&player, &sequence);
    assert_eq!(result, Err(Ok(Error::InsufficientReserve)));
}

#[test]
fn test_admin_can_withdraw_reserve() {
    let s = setup();
    let client = DiceGameClient::new(&s.env, &s.contract_id);

    let admin_balance_before = s.token.balance(&s.admin);
    let contract_balance_before = s.token.balance(&s.contract_id);
    client.withdraw(&ENTRY_FEE);
    assert_eq!(s.token.balance(&s.admin), admin_balance_before + ENTRY_FEE);
    assert_eq!(s.token.balance(&s.contract_id), contract_balance_before - ENTRY_FEE);
}

#[test]
fn test_anyone_can_deposit() {
    let s = setup();
    let client = DiceGameClient::new(&s.env, &s.contract_id);

    let contract_balance_before = s.token.balance(&s.contract_id);
    s.token_admin.mint(&s.player, &ENTRY_FEE);
    client.deposit(&s.player, &ENTRY_FEE);
    assert_eq!(
        s.token.balance(&s.contract_id),
        contract_balance_before + ENTRY_FEE
    );
}

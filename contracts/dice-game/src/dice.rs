use soroban_sdk::{Env, Vec};

// SplitMix64 finalizer (Vigna's published constants) - a canonical,
// externally-documented mixing function, not a bespoke bit-twiddle. Chosen
// specifically so the JS port has a ground truth to verify against rather
// than an invented sequence nobody outside this codebase can check.
fn mix64(mut z: u64) -> u64 {
    z ^= z >> 30;
    z = z.wrapping_mul(0xbf58476d1ce4e5b9);
    z ^= z >> 27;
    z = z.wrapping_mul(0x94d049bb133111eb);
    z ^= z >> 31;
    z
}

fn mix_key(seed: u64, roll_index: u32, die_index: u32) -> u64 {
    seed ^ (roll_index as u64).wrapping_mul(0x9E3779B97F4A7C15)
        ^ (die_index as u64).wrapping_mul(0xC2B2AE3D27D4EB4F)
}

/// Deterministic die face for a given (seed, roll_index, die_index). This
/// MUST produce byte-identical results to frontend/src/lib/diceRng.js - see
/// scripts/checkDiceParity.mjs, which is the hard gate that verifies this.
/// A keyed hash (not a stream generator) so any (roll_index, die_index)
/// pair is O(1) to compute directly, without threading state through a
/// sequence - needed both for on-chain replay and for the frontend to
/// resume mid-game from a stored hold_mask history.
pub fn die_value(seed: u64, roll_index: u32, die_index: u32) -> u32 {
    let key = mix_key(seed, roll_index, die_index);
    ((mix64(key) % 6) + 1) as u32
}

/// The initial 10-dice roll for a fresh game (roll_index = 0, nothing held).
pub fn compute_dice(env: &Env, seed: u64, roll_index: u32) -> Vec<u32> {
    let mut dice = Vec::new(env);
    for die_index in 0..10 {
        dice.push_back(die_value(seed, roll_index, die_index));
    }
    dice
}

/// Applies held_mask (bit i = die i is held) and rerolls every unheld die
/// deterministically for the given roll_index.
pub fn reroll_unheld_deterministic(
    env: &Env,
    dice: &Vec<u32>,
    held_mask: u32,
    seed: u64,
    roll_index: u32,
) -> Vec<u32> {
    let mut new_dice = Vec::new(env);
    for i in 0..dice.len() {
        if held_mask & (1 << i) != 0 {
            new_dice.push_back(dice.get(i).unwrap());
        } else {
            new_dice.push_back(die_value(seed, roll_index, i));
        }
    }
    new_dice
}

pub fn all_equal(dice: &Vec<u32>) -> bool {
    match dice.get(0) {
        Some(first) => dice.iter().all(|d| d == first),
        None => false,
    }
}

#[cfg(test)]
mod dice_sanity {
    use super::*;

    #[test]
    fn die_value_always_in_range() {
        for seed in [0u64, 1, u64::MAX, 0xAAAAAAAAAAAAAAAA] {
            for roll_index in 0..20u32 {
                for die_index in 0..10u32 {
                    let v = die_value(seed, roll_index, die_index);
                    assert!((1..=6).contains(&v), "die_value out of range: {v}");
                }
            }
        }
    }

    #[test]
    fn die_value_roughly_uniform() {
        let mut counts = [0u32; 7];
        for roll_index in 0..200u32 {
            for die_index in 0..10u32 {
                let v = die_value(0x1234_5678_9abc_def0, roll_index, die_index);
                counts[v as usize] += 1;
            }
        }
        let total = 2000u32;
        for face in 1..=6 {
            let share = counts[face] as f64 / total as f64;
            assert!(
                (0.10..0.24).contains(&share),
                "face {face} share {share} looks too skewed"
            );
        }
    }

    #[test]
    fn domain_separation_roll_vs_die_index() {
        // Check the pre-mix key, not the final 1..=6 face - two different
        // keys can legitimately land on the same face by chance (1-in-6),
        // that's not a domain-separation bug. The actual property being
        // tested is that (roll_index=1, die_index=0) and (roll_index=0,
        // die_index=1) never produce the same key just because they'd
        // sum/xor to the same naive value without distinct constants.
        assert_ne!(
            mix_key(42, 1, 0),
            mix_key(42, 0, 1),
            "roll_index/die_index are not properly domain-separated"
        );
    }
}

/// Generates the cross-language parity fixture consumed by
/// frontend/scripts/checkDiceParity.mjs. This is the ground-truth Rust
/// implementation writing out (seed, roll_index, die_index, expected)
/// tuples for the JS port in diceRng.js to be checked against - the hard
/// gate before any frontend wiring proceeds. Run via:
///   cargo test --package dice-game generate_parity_fixture -- --ignored --nocapture
#[cfg(test)]
mod fixture_gen {
    extern crate std;

    use super::die_value;
    use std::{fs, format, string::String, vec::Vec};

    #[test]
    #[ignore = "generates a checked-in fixture file; run explicitly, not on every cargo test"]
    fn generate_parity_fixture() {
        let seeds: [u64; 5] = [
            0,
            1,
            u64::MAX,
            0xAAAA_AAAA_AAAA_AAAA,
            0x1234_5678_9abc_def0,
        ];

        let mut entries: Vec<String> = Vec::new();
        for &seed in seeds.iter() {
            for roll_index in 0u32..=15 {
                for die_index in 0u32..10 {
                    let expected = die_value(seed, roll_index, die_index);
                    entries.push(format!(
                        "{{\"seed\":\"{seed}\",\"roll_index\":{roll_index},\"die_index\":{die_index},\"expected\":{expected}}}"
                    ));
                }
            }
        }

        let json = format!("[\n  {}\n]\n", entries.join(",\n  "));
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let path = format!("{manifest_dir}/fixtures/dice_parity.json");
        fs::create_dir_all(format!("{manifest_dir}/fixtures")).unwrap();
        fs::write(&path, json).unwrap();
        std::println!("Wrote parity fixture to {path}");
    }
}

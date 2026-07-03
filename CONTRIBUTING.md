# Contributing

Thanks for taking a look. This contract is a deliberately small testnet MVP
— a real on-chain dice game with entry fees and payouts, but scoped down so
there's genuine room for outside contributions rather than a finished
project.

## Before you start

1. Read [README.md](README.md) in full, especially **How it works**,
   **Cross-language parity**, and **Fairness note** — they explain
   deliberate tradeoffs, not oversights, and the parity requirement is not
   optional.
2. Check the [Roadmap / Good First Issues](README.md#roadmap--good-first-issues)
   list. If you're picking one of those up, open an issue first so two
   people don't duplicate work.
3. If you're proposing something *not* on that list (a new game mode,
   different payout mechanics, etc.), open an issue to discuss before
   writing code — this keeps the core contract focused.

## If you touch `dice.rs`

This is the one file in the repo with a hard rule: `dice.rs`'s dice logic
is duplicated in `frontend/src/lib/diceRng.js`, and the two **must** stay
byte-identical, or the frontend can show a "win" the contract's replay
won't confirm (or vice versa). Any change to `dice.rs` requires, in this
order:

1. Regenerate the fixture: `cargo test --package dice-game
   generate_parity_fixture -- --ignored --nocapture`.
2. Copy it into the frontend repo:
   `frontend/scripts/fixtures/dice_parity.json`.
3. Port your change to `diceRng.js` and run `npm run check:parity` in the
   frontend repo — it must pass.
4. Only then update `lib.rs`/`test.rs` to match, redeploy, and regenerate
   bindings.

A PR that changes `dice.rs` without an accompanying `diceRng.js` change and
a passing parity check will be asked to add both before merge.

## Development setup

```bash
# Requires the `stellar` CLI (not the old `soroban` CLI) and Rust with wasm32v1-none
cargo test                 # unit tests must pass
stellar contract build     # must produce a valid .wasm
```

## Guidelines

- **Methodology over patterns.** Prefer fixes that address a class of bug
  (e.g. "every persistent-storage access bumps TTL") over one-off patches.
- **Money math stays integer.** No floats for anything touching `entry_fee`
  or payouts — use `checked_mul`/`checked_div` and let it fail loud
  (`Error::Overflow`) rather than silently wrap or round.
- **New entrypoints need unit tests.** Follow the existing pattern in
  `src/test.rs` (a `setup()` helper registering a test SAC token +
  `mock_all_auths`, then targeted `try_*` calls asserting on `Error`
  variants).
- **Don't hardcode fairness claims.** If you touch `dice.rs` or anything
  touching `env.prng()`, keep the README's fairness disclaimer accurate —
  don't describe the game as "provably fair" unless you've actually added a
  VRF/commit-reveal scheme.
- **Checks-effects-interactions**, even though Soroban's native XLM SAC has
  no reentrant callback hooks — mutate state before calling
  `token::Client::transfer`, not after.

## Pull requests

- Keep PRs focused — one concern per PR is much easier to review than a
  bundle of unrelated changes.
- Include the `cargo test` output (or note if you added new tests) in the
  PR description.
- If your change affects the deployed testnet contract's behavior, note
  whether it requires a redeploy (most entrypoint changes do, since Soroban
  contracts aren't hot-patchable without an explicit upgrade mechanism —
  see the "Contract upgradability" item in the roadmap).

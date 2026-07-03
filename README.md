# Crypto Tenzies — Dice Game Contract (Soroban / Stellar)

A fully on-chain Tenzies-style dice game for Stellar testnet, built as
**commit-reveal + single settlement**: `start_game` charges an XLM entry fee
and commits to one random seed on-chain; the entire game (up to 15 rolls) is
then played *instantly, client-side, with zero blockchain calls per roll*
(see [../frontend](../frontend)); the player submits exactly one more
transaction, `claim`, only if they win, and the contract replays their move
sequence against the committed seed to verify it before paying out.

This replaced an earlier design where every single roll was its own
on-chain transaction (~5-6s wait each, up to 15 times a game) — that
approach was correct but felt genuinely slow to actually play. See
[How it works](#how-it-works) for the replay/verification model, and
[Fairness note](#fairness-note) for the trade-off this design accepts.

**Status: testnet MVP.** This is a deployable, tested core loop, submitted as
part of an open-source contribution project — deliberately scoped small so
there's real room for contributors. See [Roadmap / Good First
Issues](#roadmap--good-first-issues) below.

## Live deployment (testnet)

| | |
|---|---|
| Contract ID | `CCREVHBDDXGECXHXBBVGVLGDIKIDLTMCOCX2TLOZ5ES4RMZ2WXQUNM3H` |
| Explorer | [View on Stellar Expert](https://stellar.expert/explorer/testnet/contract/CCREVHBDDXGECXHXBBVGVLGDIKIDLTMCOCX2TLOZ5ES4RMZ2WXQUNM3H) |
| Network | Test SDF Network ; September 2015 |
| Entry fee | 1 XLM |
| Payout multiplier | 2x |
| Max rolls | 15 |
| Round length | 90s |

## How it works

- `initialize(admin, token, entry_fee, payout_multiplier_bps, max_rolls, round_seconds)` —
  one-time setup, called by the admin right after deploy.
- `start_game(player)` — player pays `entry_fee` in XLM; the contract
  generates one `seed: u64` via `env.prng()` and stores a `Session { seed,
  max_rolls, deadline_ts, entry_fee, payout_multiplier_bps }`. That's it —
  no dice are rolled here, and nothing else is tracked on-chain during
  play, because the entire game is a pure deterministic function of
  `(seed, the sequence of hold_masks submitted to claim)`. Only one active
  session per player; a prior session can be overwritten once its deadline
  passes.
- `claim(player, hold_mask_sequence)` — verifies a win and pays out.
  `hold_mask_sequence` is the player's full history of hold choices, one
  entry per roll they performed locally (array position *is* the roll
  index — no explicit index is ever accepted, which is what makes
  reordering or skipping a roll structurally impossible rather than merely
  inconvenient). The contract replays the exact same deterministic
  algorithm the frontend used (see `dice.rs` / `frontend/src/lib/diceRng.js`
  — these two implementations **must** stay byte-identical, see
  [Cross-language parity](#cross-language-parity-the-single-highest-risk-part)):
  starting from the initial roll, each mask is applied in order, and the
  result must be all-equal only once the sequence is fully consumed. Pays
  out `entry_fee * payout_multiplier_bps / 100` and clears the session.
- **A losing game costs no transaction at all.** If the player's local
  replay never reaches all-equal within `max_rolls`, they just never call
  `claim` — the entry fee is already forfeit, and the stale session becomes
  overwritable by their next `start_game` once `deadline_ts` passes.
- `get_session(player)` / `get_config()` — public reads, no auth required.
- `set_config(...)` — admin-only tuning. Live sessions snapshot their own
  `entry_fee`/`payout_multiplier_bps` at `start_game` time, so changing
  config mid-flight never retroactively affects an in-progress game.
- `deposit(from, amount)` / `withdraw(amount)` — house-reserve liquidity
  management. `deposit` is open to anyone; `withdraw` is admin-only. The
  contract needs a funded reserve beyond collected entry fees to cover a
  `payout_multiplier_bps > 100` payout, and `claim` explicitly checks the
  reserve rather than assuming it.

### Cross-language parity (the single highest-risk part)

`dice.rs`'s `die_value(seed, roll_index, die_index)` (a keyed SplitMix64
mix) is implemented **twice** — once here, once in
`frontend/src/lib/diceRng.js` — and the two must agree on every input. A
silent mismatch means the frontend could show a "win" the contract's
on-chain replay won't confirm, or vice versa. This is verified by a hard
gate, not just code review: `dice.rs` has an `#[ignore]`d test,
`generate_parity_fixture`, that writes `(seed, roll_index, die_index,
expected)` tuples computed by the *real Rust implementation* to
`fixtures/dice_parity.json`; the frontend's `npm run check:parity` asserts
its JS port matches every tuple. **If you change `dice.rs`, regenerate the
fixture, copy it to `frontend/scripts/fixtures/dice_parity.json`, and
re-run the parity check before touching anything downstream:**

```bash
cargo test --package dice-game generate_parity_fixture -- --ignored --nocapture
cp contracts/dice-game/fixtures/dice_parity.json ../frontend/scripts/fixtures/dice_parity.json
cd ../frontend && npm run check:parity
```

### Fairness note

Dice values come from a keyed SplitMix64 hash seeded by a single
`env.prng()` call at `start_game` — this is a fine, standard testnet
approach, but per Soroban's own docs `env.prng()` isn't suitable for
security-sensitive work (it's derived from ledger/transaction entropy, not
a verifiable randomness oracle). Separately, and more specific to this
design: since the whole game is "does a winning `hold_mask_sequence` exist
for this seed within `max_rolls`," a player can explore a fixed seed's
decision tree client-side before deciding what to submit, rather than
committing to each choice turn-by-turn as a per-roll-transaction design
would force. This can only help them *find* a winning path that already
exists for that seed — it can't change whether one exists — so it doesn't
move the underlying odds, but it's a real, deliberate trade-off. Don't
market this contract as "provably fair" without a real VRF or a stronger
commitment scheme layered on top first (see Roadmap).

### Storage

- `Config` lives in **instance** storage (small, bundled with the contract).
- Per-player `Session` lives in **persistent** storage, with its TTL bumped
  on every call that touches it (`start_game`, `claim`, `get_session`) so a
  session can't be archived out from under an in-progress game.

### Design note: `get_session` is public

Any address's session (seed, deadline, etc.) can be read by anyone, no auth
required. This is intentional, not an oversight — once a seed is committed
on-chain it isn't a secret (the whole point of this design is that both the
contract and the player compute the same dice from it), and there's no
fairness implication to exposing it.

### Design note: waiting out a loss

Because a loss never gets a settling transaction, the contract has no way
to know a session is "over" until its original `deadline_ts` passes — a
player who loses is asked to wait out the remainder of that round's timer
before `start_game` will let them begin again (this is why `round_seconds`
is tuned down to 90s in this deployment, now that rolling itself is
instant: a real player would otherwise have to sit through the *old*,
per-roll-transaction-era default of 180s). See Roadmap for a cleaner fix.

## Development

Requires the [`stellar` CLI](https://developers.stellar.org/docs/tools/stellar-cli)
(not the older `soroban` CLI) and Rust with the `wasm32v1-none` target.

```bash
cargo test                 # unit tests (soroban-sdk testutils, mocked auth)
stellar contract build     # -> target/wasm32v1-none/release/dice_game.wasm
```

### Deploying your own instance

```bash
stellar keys generate my-admin --network testnet --fund

stellar contract deploy \
  --wasm target/wasm32v1-none/release/dice_game.wasm \
  --source my-admin --network testnet -- 

TOKEN=$(stellar contract id asset --asset native --network testnet)

stellar contract invoke --id <CONTRACT_ID> --source my-admin --network testnet -- \
  initialize --admin <ADMIN_ADDRESS> --token $TOKEN \
  --entry_fee 10000000 --payout_multiplier_bps 200 --max_rolls 15 --round_seconds 90

# Fund the house reserve so payouts above 1x entry fee can be covered:
stellar contract invoke --id <CONTRACT_ID> --source my-admin --network testnet -- \
  deposit --from <ADMIN_ADDRESS> --amount 100000000
```

Then regenerate frontend bindings against the new contract id:

```bash
stellar contract bindings typescript --network testnet --contract-id <CONTRACT_ID> \
  --output-dir ../frontend/src/packages/dice-game-bindings --overwrite
```

## Roadmap

These were deliberately deferred to keep this an honest MVP rather than a
half-finished "everything" build:

- **`forfeit(player)` entrypoint** — lets a player who's locally lost close
  out their session immediately (for a transaction, trading the "losses are
  free" property for not having to wait out `deadline_ts`) instead of being
  forced to wait. See "Design note: waiting out a loss" above.
- **`min_reserve` floor on `withdraw`** — prevent an admin from draining the
  contract below what's needed to cover pending winners.
- **Stats / leaderboard** — index `start_game`/`claim` events off-chain (or
  add contract events) to build a real leaderboard.
- **Emergency pause** — an admin-toggleable `paused` flag checked at the top
  of `start_game`/`claim`, for halting new games without a redeploy if a
  bug is found post-launch.
- **Real fairness upgrade** — a real VRF/oracle-backed randomness source,
  and/or a stronger commitment scheme that removes the "explore the
  decision tree before submitting" trade-off described above.
- **Contract upgradability** — add an admin-gated `upgrade` entrypoint
  (`env.deployer().update_current_contract_wasm(...)`) so future fixes don't
  require a fresh deploy + re-initialize + re-fund cycle like this one did
  (twice, so far).

Open an issue or just send a PR — the contract is small (~250 lines across
`lib.rs`/`types.rs`/`storage.rs`/`dice.rs`/`errors.rs`), so it's a
reasonable first read.

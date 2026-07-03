use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    SessionActive = 3,
    NoActiveSession = 4,
    SessionExpired = 5,
    MaxRollsReached = 6,
    NotWon = 7,
    Overflow = 8,
    InsufficientReserve = 9,
    InvalidHoldMask = 10,
    InvalidRollSequence = 11,
}

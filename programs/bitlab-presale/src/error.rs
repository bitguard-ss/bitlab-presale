use anchor_lang::prelude::*;

#[error_code]
pub enum PresaleError {
    #[msg("Sale has not started")]
    NotStarted,
    #[msg("Sale is not active")]
    NotActive,
    #[msg("Sale is paused")]
    Paused,
    #[msg("Sale is sold out")]
    SoldOut,
    #[msg("Sale has ended")]
    Ended,
    #[msg("Scheduled window has closed")]
    ScheduleEnded,
    #[msg("Configuration is not finalized")]
    ConfigOpen,
    #[msg("Configuration is frozen")]
    ConfigFrozen,
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Pending authority must accept")]
    PendingAuthority,
    #[msg("Payment asset is disabled")]
    AssetDisabled,
    #[msg("Unapproved mint")]
    UnapprovedMint,
    #[msg("Invalid token account")]
    InvalidTokenAccount,
    #[msg("Invalid vault")]
    InvalidVault,
    #[msg("Invalid oracle account")]
    InvalidOracle,
    #[msg("Oracle price is stale")]
    StaleOracle,
    #[msg("Oracle confidence is too wide")]
    OracleConfidence,
    #[msg("Oracle is not in trading status")]
    InvalidOracleStatus,
    #[msg("Oracle staleness window is out of bounds")]
    OracleStalenessBounds,
    #[msg("Zero payment")]
    ZeroPayment,
    #[msg("Below minimum purchase")]
    BelowMinimum,
    #[msg("Above maximum purchase")]
    AboveMaximum,
    #[msg("Wallet contribution cap exceeded")]
    WalletCap,
    #[msg("Would exceed remaining BITLAB inventory")]
    ExceedsRemaining,
    #[msg("Slippage exceeded: BITLAB below minimum accepted")]
    SlippageExceeded,
    #[msg("Insufficient vault balance")]
    InsufficientVault,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Zero price")]
    ZeroPrice,
    #[msg("Already finalized")]
    AlreadyFinalized,
    #[msg("Sale is not closed")]
    NotClosed,
    #[msg("Cannot resume a terminated sale")]
    Terminal,
    #[msg("Invalid destination")]
    InvalidDestination,
    #[msg("BITLAB inventory cannot be recovered through payment emergency")]
    BitlabProtected,
    #[msg("Wrong token program")]
    WrongTokenProgram,
    #[msg("Account mismatch")]
    AccountMismatch,
    #[msg("Invalid or default authority")]
    InvalidAuthority,
    #[msg("Vault is not owned by the presale state")]
    VaultOwner,
    #[msg("BITLAB mint or freeze authority is still active")]
    MintAuthorityActive,
    #[msg("Purchase limit invariant failed")]
    LimitInvariant,
    #[msg("Fee-on-transfer or unexpected token behavior")]
    FeeOnTransfer,
    #[msg("Would drop the SOL vault below rent exemption")]
    RentExempt,
    #[msg("Emergency stop is still engaged")]
    EmergencyEngaged,
    #[msg("Emergency stop is not engaged")]
    EmergencyNotEngaged,
    #[msg("SOL vault must be a system-owned empty PDA")]
    InvalidSolVault,
    #[msg("Sale start timestamp is invalid")]
    InvalidStartTs,
}

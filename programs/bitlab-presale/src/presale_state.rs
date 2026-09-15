use anchor_lang::prelude::*;
use crate::constants::ROUND_COUNT;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum SaleStatus {
    NotStarted,
    Active,
    Paused,
    SoldOut,
    Ended,
    Finalized,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum EndReason {
    None,
    SoldOut,
    AdminTerminated,
    Schedule,
}

#[account]
#[derive(InitSpace)]
pub struct PresaleState {
    pub bump: u8,
    pub sol_vault_bump: u8,
    pub config_finalized: bool,
    pub status: SaleStatus,
    pub end_reason: EndReason,
    pub emergency_stop: bool,

    pub ops_authority: Pubkey,
    pub pending_ops_authority: Pubkey,
    pub treasury_authority: Pubkey,
    pub pending_treasury_authority: Pubkey,
    pub emergency_authority: Pubkey,
    pub pending_emergency_authority: Pubkey,

    pub treasury: Pubkey,
    pub unsold_destination: Pubkey,
    pub bitlab_mint: Pubkey,
    pub usdc_mint: Pubkey,
    pub usdt_mint: Pubkey,
    pub bitlab_vault: Pubkey,
    pub usdc_vault: Pubkey,
    pub usdt_vault: Pubkey,
    pub sol_vault: Pubkey,
    pub oracle: Pubkey,

    pub allocation: u64,
    pub total_sold: u64,
    pub unique_buyers: u64,
    pub purchase_count: u64,
    pub sale_start_ts: i64,
    pub round_duration_secs: i64,
    pub round_count: u8,
    pub prices_e9: [u64; ROUND_COUNT as usize],
    pub sol_enabled: bool,
    pub usdc_enabled: bool,
    pub usdt_enabled: bool,
    pub min_purchase_usd_e6: u64,
    pub max_purchase_usd_e6: u64,
    pub max_wallet_usd_e6: u64,
    pub oracle_max_staleness_secs: i64,
    pub paused_at: i64,
    pub ended_at: i64,
    pub finalized_at: i64,

    /// Lifetime payments received (forwarded to `treasury` in the same buy tx).
    pub sol_received: u64,
    pub usdc_received: u64,
    pub usdt_received: u64,
}

#[account]
#[derive(InitSpace)]
pub struct BuyerAccount {
    pub bump: u8,
    pub buyer: Pubkey,
    pub presale: Pubkey,
    pub bitlab_purchased: u64,
    pub usd_contributed_e6: u64,
    pub purchase_count: u32,
}

#[event]
pub struct LockPurchaseEvent {
    pub buyer: Pubkey,
    pub payment_asset: u8, // 0 SOL, 1 USDC, 2 USDT
    pub payment_amount: u64,
    pub usd_value_e6: u64,
    pub round: u8,
    pub bitlab_price_e9: u64,
    pub bitlab_amount: u64,
    pub timestamp: i64,
}

#[event]
pub struct StatusEvent {
    pub status: SaleStatus,
    pub reason: EndReason,
    pub timestamp: i64,
}

#[event]
pub struct WithdrawEvent {
    pub asset: u8,
    pub amount: u64,
    pub destination: Pubkey,
    pub emergency: bool,
    pub timestamp: i64,
}

#[event]
pub struct AuthorityEvent {
    pub role: u8, // 0 ops, 1 treasury, 2 emergency
    pub proposed: bool,
    pub authority: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct TreasuryEvent {
    pub treasury: Pubkey,
    pub timestamp: i64,
}

#[event]
pub struct ConfigEvent {
    pub sale_start_ts: i64,
    pub timestamp: i64,
}

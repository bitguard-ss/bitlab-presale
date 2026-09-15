use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

pub mod constants;
pub mod error;
pub mod presale_state;

use constants::*;
use error::PresaleError;
use presale_state::*;

declare_id!("Bit7n73gYrnXMPXeeA3yiuoUsLRZ5ztXqzCZrduLaT5n");

#[cfg(not(feature = "no-entrypoint"))]
solana_security_txt::security_txt! {
    name: "BitLab Coin Presale",
    project_url: "https://bbspectrum.com",
    contacts: "email:m.williams@bbspectrum.com,email:operation@bbspectrum.com",
    policy: "https://bbspectrum.com/restrictions-disclaimer",
    preferred_languages: "en",
    source_code: "https://bbspectrum.com",
    auditors: "Independent audit required before public sale"
}

#[program]
pub mod bitlab_presale {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
        assert_pubkey_set(&args.ops_authority)?;
        assert_pubkey_set(&args.treasury_authority)?;
        assert_pubkey_set(&args.emergency_authority)?;
        assert_pubkey_set(&args.treasury)?;
        assert_pubkey_set(&args.unsold_destination)?;
        require_keys_eq!(ctx.accounts.payer.key(), args.ops_authority, PresaleError::Unauthorized);
        require_keys_eq!(args.bitlab_mint, BITLAB_MINT, PresaleError::UnapprovedMint);
        require_keys_eq!(args.oracle, PYTH_SOL_USD, PresaleError::InvalidOracle);
        require!(args.sale_start_ts > 0, PresaleError::InvalidStartTs);
        require!(ctx.accounts.bitlab_mint.decimals == BITLAB_DECIMALS, PresaleError::AccountMismatch);
        require!(ctx.accounts.usdc_mint.decimals == STABLE_DECIMALS, PresaleError::AccountMismatch);
        require!(ctx.accounts.usdt_mint.decimals == STABLE_DECIMALS, PresaleError::AccountMismatch);
        require!(ctx.accounts.bitlab_mint.mint_authority.is_none(), PresaleError::MintAuthorityActive);
        require!(ctx.accounts.bitlab_mint.freeze_authority.is_none(), PresaleError::MintAuthorityActive);
        assert_limit_invariant(args.min_purchase_usd_e6, args.max_purchase_usd_e6, args.max_wallet_usd_e6)?;
        let staleness = assert_staleness_bounds(args.oracle_max_staleness_secs)?;

        require!(ctx.accounts.sol_vault.data_is_empty(), PresaleError::InvalidSolVault);
        let rent = Rent::get()?.minimum_balance(0);
        if ctx.accounts.sol_vault.lamports() < rent {
            system_program::transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.payer.to_account_info(),
                        to: ctx.accounts.sol_vault.to_account_info(),
                    },
                ),
                rent.saturating_sub(ctx.accounts.sol_vault.lamports()),
            )?;
        }

        let s = &mut ctx.accounts.state;
        s.bump = ctx.bumps.state;
        s.sol_vault_bump = ctx.bumps.sol_vault;
        s.config_finalized = false;
        s.status = SaleStatus::NotStarted;
        s.end_reason = EndReason::None;
        s.emergency_stop = false;
        s.ops_authority = args.ops_authority;
        s.pending_ops_authority = Pubkey::default();
        s.treasury_authority = args.treasury_authority;
        s.pending_treasury_authority = Pubkey::default();
        s.emergency_authority = args.emergency_authority;
        s.pending_emergency_authority = Pubkey::default();
        s.treasury = args.treasury;
        s.unsold_destination = args.unsold_destination;
        s.bitlab_mint = BITLAB_MINT;
        s.usdc_mint = USDC_MINT;
        s.usdt_mint = USDT_MINT;
        s.bitlab_vault = ctx.accounts.bitlab_vault.key();
        s.usdc_vault = ctx.accounts.usdc_vault.key();
        s.usdt_vault = ctx.accounts.usdt_vault.key();
        s.sol_vault = ctx.accounts.sol_vault.key();
        s.oracle = PYTH_SOL_USD;
        s.allocation = PRESALE_ALLOCATION;
        s.total_sold = 0;
        s.unique_buyers = 0;
        s.purchase_count = 0;
        s.sale_start_ts = args.sale_start_ts;
        s.round_duration_secs = ROUND_DURATION_SECS;
        s.round_count = ROUND_COUNT;
        s.prices_e9 = ROUND_PRICES_E9;
        s.sol_enabled = true;
        s.usdc_enabled = true;
        s.usdt_enabled = true;
        s.min_purchase_usd_e6 = args.min_purchase_usd_e6;
        s.max_purchase_usd_e6 = args.max_purchase_usd_e6;
        s.max_wallet_usd_e6 = args.max_wallet_usd_e6;
        s.oracle_max_staleness_secs = staleness;
        s.paused_at = 0;
        s.ended_at = 0;
        s.finalized_at = 0;
        s.sol_received = 0;
        s.usdc_received = 0;
        s.usdt_received = 0;
        emit!(ConfigEvent {
            sale_start_ts: args.sale_start_ts,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }

    pub fn finalize_config(ctx: Context<OpsOnly>) -> Result<()> {
        let s = &mut ctx.accounts.state;
        require!(!s.config_finalized, PresaleError::ConfigFrozen);
        s.config_finalized = true;
        emit!(ConfigEvent {
            sale_start_ts: s.sale_start_ts,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }

    pub fn start_sale(ctx: Context<OpsOnly>, sale_start_ts: i64) -> Result<()> {
        require!(sale_start_ts > 0, PresaleError::InvalidStartTs);
        let s = &mut ctx.accounts.state;
        require!(s.config_finalized, PresaleError::ConfigOpen);
        require!(s.purchase_count == 0, PresaleError::ConfigFrozen);
        require!(s.status == SaleStatus::NotStarted, PresaleError::ConfigFrozen);
        require!(!s.emergency_stop, PresaleError::EmergencyEngaged);
        s.sale_start_ts = sale_start_ts;
        s.status = SaleStatus::Active;
        emit!(StatusEvent {
            status: s.status,
            reason: s.end_reason,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }

    pub fn pause(ctx: Context<OpsOnly>) -> Result<()> {
        let s = &mut ctx.accounts.state;
        require!(s.status == SaleStatus::Active, PresaleError::NotActive);
        s.status = SaleStatus::Paused;
        s.paused_at = Clock::get()?.unix_timestamp;
        emit!(StatusEvent {
            status: s.status,
            reason: s.end_reason,
            timestamp: s.paused_at,
        });
        Ok(())
    }

    pub fn resume(ctx: Context<OpsOnly>) -> Result<()> {
        let s = &mut ctx.accounts.state;
        require!(s.status == SaleStatus::Paused, PresaleError::Paused);
        require!(s.end_reason == EndReason::None, PresaleError::Terminal);
        require!(!s.emergency_stop, PresaleError::EmergencyEngaged);
        s.status = SaleStatus::Active;
        emit!(StatusEvent {
            status: s.status,
            reason: s.end_reason,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }

    pub fn clear_emergency(ctx: Context<EmergencyOnly>) -> Result<()> {
        let s = &mut ctx.accounts.state;
        require!(s.emergency_stop, PresaleError::EmergencyNotEngaged);
        s.emergency_stop = false;
        emit!(StatusEvent {
            status: s.status,
            reason: s.end_reason,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }

    pub fn end_presale(ctx: Context<OpsOnly>) -> Result<()> {
        let s = &mut ctx.accounts.state;
        require!(
            s.status != SaleStatus::Ended
                && s.status != SaleStatus::Finalized
                && s.status != SaleStatus::SoldOut,
            PresaleError::Ended
        );
        s.status = SaleStatus::Ended;
        s.end_reason = EndReason::AdminTerminated;
        s.ended_at = Clock::get()?.unix_timestamp;
        emit!(StatusEvent {
            status: s.status,
            reason: s.end_reason,
            timestamp: s.ended_at,
        });
        Ok(())
    }

    pub fn set_asset_enabled(ctx: Context<OpsOnly>, asset: u8, enabled: bool) -> Result<()> {
        let s = &mut ctx.accounts.state;
        require!(s.status != SaleStatus::Finalized, PresaleError::AlreadyFinalized);
        match asset {
            0 => s.sol_enabled = enabled,
            1 => s.usdc_enabled = enabled,
            2 => s.usdt_enabled = enabled,
            _ => return err!(PresaleError::AssetDisabled),
        }
        Ok(())
    }

    pub fn set_purchase_limits(
        ctx: Context<OpsOnly>,
        min_usd_e6: u64,
        max_usd_e6: u64,
        max_wallet_usd_e6: u64,
    ) -> Result<()> {
        let s = &mut ctx.accounts.state;
        require!(!s.config_finalized, PresaleError::ConfigFrozen);
        assert_limit_invariant(min_usd_e6, max_usd_e6, max_wallet_usd_e6)?;
        s.min_purchase_usd_e6 = min_usd_e6;
        s.max_purchase_usd_e6 = max_usd_e6;
        s.max_wallet_usd_e6 = max_wallet_usd_e6;
        Ok(())
    }

    pub fn set_oracle(ctx: Context<OpsOnly>, oracle: Pubkey, max_staleness_secs: i64) -> Result<()> {
        let s = &mut ctx.accounts.state;
        require!(!s.config_finalized, PresaleError::ConfigFrozen);
        require_keys_eq!(oracle, PYTH_SOL_USD, PresaleError::InvalidOracle);
        s.oracle = oracle;
        s.oracle_max_staleness_secs = assert_staleness_bounds(max_staleness_secs)?;
        Ok(())
    }

    pub fn propose_ops_authority(ctx: Context<OpsOnly>, next: Pubkey) -> Result<()> {
        assert_pubkey_set(&next)?;
        require_keys_neq!(next, ctx.accounts.state.ops_authority, PresaleError::InvalidAuthority);
        ctx.accounts.state.pending_ops_authority = next;
        emit!(AuthorityEvent {
            role: 0,
            proposed: true,
            authority: next,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }
    pub fn accept_ops_authority(ctx: Context<AcceptOps>) -> Result<()> {
        let s = &mut ctx.accounts.state;
        assert_pubkey_set(&s.pending_ops_authority)?;
        require_keys_eq!(s.pending_ops_authority, ctx.accounts.new_authority.key(), PresaleError::PendingAuthority);
        s.ops_authority = s.pending_ops_authority;
        s.pending_ops_authority = Pubkey::default();
        emit!(AuthorityEvent {
            role: 0,
            proposed: false,
            authority: s.ops_authority,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }
    pub fn propose_treasury_authority(ctx: Context<TreasuryOnly>, next: Pubkey) -> Result<()> {
        assert_pubkey_set(&next)?;
        require_keys_neq!(next, ctx.accounts.state.treasury_authority, PresaleError::InvalidAuthority);
        ctx.accounts.state.pending_treasury_authority = next;
        emit!(AuthorityEvent {
            role: 1,
            proposed: true,
            authority: next,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }
    pub fn accept_treasury_authority(ctx: Context<AcceptTreasury>) -> Result<()> {
        let s = &mut ctx.accounts.state;
        assert_pubkey_set(&s.pending_treasury_authority)?;
        require_keys_eq!(s.pending_treasury_authority, ctx.accounts.new_authority.key(), PresaleError::PendingAuthority);
        s.treasury_authority = s.pending_treasury_authority;
        s.pending_treasury_authority = Pubkey::default();
        emit!(AuthorityEvent {
            role: 1,
            proposed: false,
            authority: s.treasury_authority,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }
    pub fn propose_emergency_authority(ctx: Context<EmergencyOnly>, next: Pubkey) -> Result<()> {
        assert_pubkey_set(&next)?;
        require_keys_neq!(next, ctx.accounts.state.emergency_authority, PresaleError::InvalidAuthority);
        ctx.accounts.state.pending_emergency_authority = next;
        emit!(AuthorityEvent {
            role: 2,
            proposed: true,
            authority: next,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }
    pub fn accept_emergency_authority(ctx: Context<AcceptEmergency>) -> Result<()> {
        let s = &mut ctx.accounts.state;
        assert_pubkey_set(&s.pending_emergency_authority)?;
        require_keys_eq!(s.pending_emergency_authority, ctx.accounts.new_authority.key(), PresaleError::PendingAuthority);
        s.emergency_authority = s.pending_emergency_authority;
        s.pending_emergency_authority = Pubkey::default();
        emit!(AuthorityEvent {
            role: 2,
            proposed: false,
            authority: s.emergency_authority,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }

    pub fn set_treasury(ctx: Context<TreasuryOnly>, treasury: Pubkey) -> Result<()> {
        assert_pubkey_set(&treasury)?;
        require_keys_neq!(treasury, ctx.accounts.state.treasury, PresaleError::InvalidAuthority);
        ctx.accounts.state.treasury = treasury;
        emit!(TreasuryEvent {
            treasury,
            timestamp: Clock::get()?.unix_timestamp,
        });
        Ok(())
    }

    pub fn emergency_stop(ctx: Context<EmergencyOnly>) -> Result<()> {
        let s = &mut ctx.accounts.state;
        require!(s.status != SaleStatus::Finalized, PresaleError::AlreadyFinalized);
        s.emergency_stop = true;
        s.status = SaleStatus::Paused;
        s.paused_at = Clock::get()?.unix_timestamp;
        emit!(StatusEvent {
            status: s.status,
            reason: s.end_reason,
            timestamp: s.paused_at,
        });
        Ok(())
    }

    pub fn buy_with_sol(ctx: Context<BuySol>, lamports: u64, min_bitlab: u64) -> Result<()> {
        require!(lamports > 0, PresaleError::ZeroPayment);
        let now = Clock::get()?.unix_timestamp;
        let slot = Clock::get()?.slot;
        let presale = ctx.accounts.state.key();
        let state_bump = ctx.accounts.state.bump;
        let buyer_key = ctx.accounts.buyer.key();
        let buyer_bump = ctx.bumps.buyer_state;
        sync_status(&mut ctx.accounts.state, now)?;
        let (round, price_e9) = assert_can_buy(&ctx.accounts.state, now, 0)?;

        let (pyth_price, pyth_expo, _pyth_ts) = read_pyth_price(
            ctx.accounts.oracle.as_ref(),
            now,
            slot,
            ctx.accounts.state.oracle_max_staleness_secs,
        )?;

        let usd_e6 = usd_e6_from_sol(lamports, pyth_price, pyth_expo)?;
        enforce_limits(&ctx.accounts.state, &ctx.accounts.buyer_state, usd_e6)?;
        let bitlab = bitlab_from_usd_e6(usd_e6, price_e9)?;
        require!(bitlab >= min_bitlab, PresaleError::SlippageExceeded);
        require!(ctx.accounts.bitlab_vault.amount >= bitlab, PresaleError::InsufficientVault);
        bind_buyer(&mut ctx.accounts.buyer_state, buyer_bump, buyer_key, presale)?;
        deliver_bitlab(
            &mut ctx.accounts.state,
            &mut ctx.accounts.buyer_state,
            presale,
            buyer_key,
            bitlab,
            usd_e6,
            now,
        )?;
        ctx.accounts.state.sol_received = ctx.accounts.state
            .sol_received
            .checked_add(lamports)
            .ok_or(PresaleError::MathOverflow)?;

        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.buyer.to_account_info(),
                    to: ctx.accounts.treasury.to_account_info(),
                },
            ),
            lamports,
        )?;

        transfer_bitlab(
            &ctx.accounts.token_program,
            &ctx.accounts.bitlab_vault,
            &ctx.accounts.buyer_bitlab,
            &ctx.accounts.state.to_account_info(),
            state_bump,
            bitlab,
        )?;

        emit!(LockPurchaseEvent {
            buyer: buyer_key,
            payment_asset: 0,
            payment_amount: lamports,
            usd_value_e6: usd_e6,
            round: round + 1,
            bitlab_price_e9: price_e9,
            bitlab_amount: bitlab,
            timestamp: now,
        });
        Ok(())
    }

    pub fn buy_with_usdc(ctx: Context<BuyUsdc>, amount: u64, min_bitlab: u64) -> Result<()> {
        let bump = ctx.bumps.buyer_state;
        let BuyUsdc {
            ref buyer,
            ref mut state,
            ref mut buyer_state,
            ref bitlab_vault,
            ref buyer_bitlab,
            ref buyer_usdc,
            ref mut treasury_usdc,
            ref usdc_mint,
            ref token_program,
            system_program: _,
        } = ctx.accounts;
        buy_stable(
            state,
            buyer_state,
            buyer,
            bitlab_vault,
            buyer_bitlab,
            buyer_usdc,
            treasury_usdc,
            usdc_mint,
            token_program,
            bump,
            amount,
            min_bitlab,
            1,
        )
    }

    pub fn buy_with_usdt(ctx: Context<BuyUsdt>, amount: u64, min_bitlab: u64) -> Result<()> {
        let bump = ctx.bumps.buyer_state;
        let BuyUsdt {
            ref buyer,
            ref mut state,
            ref mut buyer_state,
            ref bitlab_vault,
            ref buyer_bitlab,
            ref buyer_usdt,
            ref mut treasury_usdt,
            ref usdt_mint,
            ref token_program,
            system_program: _,
        } = ctx.accounts;
        buy_stable(
            state,
            buyer_state,
            buyer,
            bitlab_vault,
            buyer_bitlab,
            buyer_usdt,
            treasury_usdt,
            usdt_mint,
            token_program,
            bump,
            amount,
            min_bitlab,
            2,
        )
    }

    pub fn withdraw_sol(ctx: Context<WithdrawSol>, amount: u64) -> Result<()> {
        withdraw_sol_inner(
            &ctx.accounts.state,
            ctx.accounts.sol_vault.as_ref(),
            ctx.accounts.destination.as_ref(),
            &ctx.accounts.system_program,
            amount,
            false,
        )
    }
    pub fn withdraw_usdc(ctx: Context<WithdrawUsdc>, amount: u64) -> Result<()> {
        withdraw_stable(
            &ctx.accounts.state,
            &ctx.accounts.usdc_vault,
            &ctx.accounts.destination,
            &ctx.accounts.token_program,
            amount,
            false,
            1,
        )
    }
    pub fn withdraw_usdt(ctx: Context<WithdrawUsdt>, amount: u64) -> Result<()> {
        withdraw_stable(
            &ctx.accounts.state,
            &ctx.accounts.usdt_vault,
            &ctx.accounts.destination,
            &ctx.accounts.token_program,
            amount,
            false,
            2,
        )
    }

    pub fn emergency_recover_sol(ctx: Context<EmergencyWithdrawSol>, amount: u64) -> Result<()> {
        require!(ctx.accounts.state.emergency_stop, PresaleError::EmergencyNotEngaged);
        withdraw_sol_inner(
            &ctx.accounts.state,
            ctx.accounts.sol_vault.as_ref(),
            ctx.accounts.destination.as_ref(),
            &ctx.accounts.system_program,
            amount,
            true,
        )
    }
    pub fn emergency_recover_usdc(ctx: Context<EmergencyWithdrawUsdc>, amount: u64) -> Result<()> {
        require!(ctx.accounts.state.emergency_stop, PresaleError::EmergencyNotEngaged);
        withdraw_stable(
            &ctx.accounts.state,
            &ctx.accounts.usdc_vault,
            &ctx.accounts.destination,
            &ctx.accounts.token_program,
            amount,
            true,
            1,
        )
    }
    pub fn emergency_recover_usdt(ctx: Context<EmergencyWithdrawUsdt>, amount: u64) -> Result<()> {
        require!(ctx.accounts.state.emergency_stop, PresaleError::EmergencyNotEngaged);
        withdraw_stable(
            &ctx.accounts.state,
            &ctx.accounts.usdt_vault,
            &ctx.accounts.destination,
            &ctx.accounts.token_program,
            amount,
            true,
            2,
        )
    }

    pub fn finalize_sale(ctx: Context<FinalizeSale>) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let state_bump = ctx.accounts.state.bump;
        {
            let s = &mut ctx.accounts.state;
            sync_status(s, now)?;
            require!(s.status != SaleStatus::Finalized, PresaleError::AlreadyFinalized);
            require!(
                s.status == SaleStatus::Ended || s.status == SaleStatus::SoldOut,
                PresaleError::NotClosed
            );
            s.status = SaleStatus::Finalized;
            s.finalized_at = now;
        }
        let remaining = ctx.accounts.bitlab_vault.amount;
        if remaining > 0 {
            transfer_bitlab(
                &ctx.accounts.token_program,
                &ctx.accounts.bitlab_vault,
                &ctx.accounts.unsold_destination,
                &ctx.accounts.state.to_account_info(),
                state_bump,
                remaining,
            )?;
        }
        emit!(StatusEvent {
            status: SaleStatus::Finalized,
            reason: ctx.accounts.state.end_reason,
            timestamp: now,
        });
        Ok(())
    }
}

fn bind_buyer(buyer: &mut BuyerAccount, bump: u8, buyer_key: Pubkey, presale: Pubkey) -> Result<()> {
    if buyer.purchase_count == 0 {
        buyer.bump = bump;
        buyer.buyer = buyer_key;
        buyer.presale = presale;
    } else {
        require_eq!(buyer.bump, bump, PresaleError::AccountMismatch);
        require_keys_eq!(buyer.buyer, buyer_key, PresaleError::AccountMismatch);
        require_keys_eq!(buyer.presale, presale, PresaleError::AccountMismatch);
    }
    Ok(())
}

fn buy_stable<'info>(
    state: &mut Account<'info, PresaleState>,
    buyer_state: &mut Account<'info, BuyerAccount>,
    buyer: &Signer<'info>,
    bitlab_vault: &Account<'info, TokenAccount>,
    buyer_bitlab: &Account<'info, TokenAccount>,
    buyer_stable: &Account<'info, TokenAccount>,
    treasury_ata: &mut Account<'info, TokenAccount>,
    payment_mint: &Account<'info, Mint>,
    token_program: &Program<'info, Token>,
    buyer_bump: u8,
    amount: u64,
    min_bitlab: u64,
    asset: u8,
) -> Result<()> {
    require!(amount > 0, PresaleError::ZeroPayment);
    let now = Clock::get()?.unix_timestamp;
    let presale = state.key();
    let state_bump = state.bump;
    let buyer_key = buyer.key();
    sync_status(state, now)?;
    let (round, price_e9) = assert_can_buy(state, now, asset)?;
    require_keys_eq!(buyer_stable.owner, buyer_key, PresaleError::InvalidTokenAccount);
    require_keys_eq!(buyer_stable.mint, payment_mint.key(), PresaleError::InvalidTokenAccount);
    require_keys_eq!(treasury_ata.mint, payment_mint.key(), PresaleError::InvalidTokenAccount);
    require_keys_eq!(treasury_ata.owner, state.treasury, PresaleError::InvalidDestination);
    let expected_mint = if asset == 1 { state.usdc_mint } else { state.usdt_mint };
    require_keys_eq!(payment_mint.key(), expected_mint, PresaleError::UnapprovedMint);
    require_keys_eq!(buyer_bitlab.mint, state.bitlab_mint, PresaleError::InvalidTokenAccount);
    require_keys_eq!(buyer_bitlab.owner, buyer_key, PresaleError::InvalidTokenAccount);
    require_keys_eq!(bitlab_vault.owner, presale, PresaleError::VaultOwner);
    require_keys_eq!(bitlab_vault.mint, BITLAB_MINT, PresaleError::UnapprovedMint);
    require_keys_eq!(token_program.key(), SPL_TOKEN_PROGRAM, PresaleError::WrongTokenProgram);

    let usd_e6 = amount;
    enforce_limits(state, buyer_state, usd_e6)?;
    let bitlab = bitlab_from_usd_e6(usd_e6, price_e9)?;
    require!(bitlab >= min_bitlab, PresaleError::SlippageExceeded);
    require!(bitlab_vault.amount >= bitlab, PresaleError::InsufficientVault);
    bind_buyer(buyer_state, buyer_bump, buyer_key, presale)?;
    deliver_bitlab(state, buyer_state, presale, buyer_key, bitlab, usd_e6, now)?;
    if asset == 1 {
        state.usdc_received = state.usdc_received.checked_add(amount).ok_or(PresaleError::MathOverflow)?;
    } else {
        state.usdt_received = state.usdt_received.checked_add(amount).ok_or(PresaleError::MathOverflow)?;
    }

    let before = treasury_ata.amount;
    token::transfer(
        CpiContext::new(
            token_program.to_account_info(),
            Transfer {
                from: buyer_stable.to_account_info(),
                to: treasury_ata.to_account_info(),
                authority: buyer.to_account_info(),
            },
        ),
        amount,
    )?;
    treasury_ata.reload()?;
    require!(
        treasury_ata.amount.saturating_sub(before) == amount,
        PresaleError::FeeOnTransfer
    );
    transfer_bitlab(
        token_program,
        bitlab_vault,
        buyer_bitlab,
        &state.to_account_info(),
        state_bump,
        bitlab,
    )?;
    emit!(LockPurchaseEvent {
        buyer: buyer_key,
        payment_asset: asset,
        payment_amount: amount,
        usd_value_e6: usd_e6,
        round: round + 1,
        bitlab_price_e9: price_e9,
        bitlab_amount: bitlab,
        timestamp: now,
    });
    Ok(())
}

fn withdraw_sol_inner<'info>(
    state: &Account<'info, PresaleState>,
    sol_vault: &AccountInfo<'info>,
    destination: &AccountInfo<'info>,
    system_program: &Program<'info, System>,
    amount: u64,
    emergency: bool,
) -> Result<()> {
    require!(amount > 0, PresaleError::ZeroPayment);
    require_keys_eq!(destination.key(), state.treasury, PresaleError::InvalidDestination);
    require_keys_eq!(sol_vault.key(), state.sol_vault, PresaleError::InvalidVault);
    require_keys_eq!(*sol_vault.owner, system_program::ID, PresaleError::InvalidSolVault);
    let rent = Rent::get()?.minimum_balance(0);
    require!(sol_vault.lamports().saturating_sub(amount) >= rent, PresaleError::RentExempt);
    let bump = state.sol_vault_bump;
    let seeds: &[&[u8]] = &[SOL_VAULT_SEED, &[bump]];
    let binding = [seeds];
    system_program::transfer(
        CpiContext::new_with_signer(
            system_program.to_account_info(),
            system_program::Transfer {
                from: sol_vault.to_account_info(),
                to: destination.to_account_info(),
            },
            &binding,
        ),
        amount,
    )?;
    emit!(WithdrawEvent {
        asset: 0,
        amount,
        destination: destination.key(),
        emergency,
        timestamp: Clock::get()?.unix_timestamp,
    });
    Ok(())
}

fn withdraw_stable<'info>(
    state: &Account<'info, PresaleState>,
    vault: &Account<'info, TokenAccount>,
    destination: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
    amount: u64,
    emergency: bool,
    asset: u8,
) -> Result<()> {
    require!(amount > 0, PresaleError::ZeroPayment);
    require_keys_eq!(destination.owner, state.treasury, PresaleError::InvalidDestination);
    require_keys_eq!(vault.mint, destination.mint, PresaleError::InvalidVault);
    require_keys_eq!(vault.owner, state.key(), PresaleError::VaultOwner);
    require!(vault.amount >= amount, PresaleError::InsufficientVault);
    require_keys_neq!(vault.mint, state.bitlab_mint, PresaleError::BitlabProtected);
    transfer_signed_tokens(
        token_program,
        vault,
        destination,
        &state.to_account_info(),
        state.bump,
        amount,
    )?;
    emit!(WithdrawEvent {
        asset,
        amount,
        destination: destination.owner,
        emergency,
        timestamp: Clock::get()?.unix_timestamp,
    });
    Ok(())
}

fn assert_can_buy(state: &PresaleState, now: i64, asset: u8) -> Result<(u8, u64)> {
    require!(state.config_finalized, PresaleError::ConfigOpen);
    require!(!state.emergency_stop, PresaleError::Paused);
    require!(state.status == SaleStatus::Active, PresaleError::NotActive);
    match asset {
        0 => require!(state.sol_enabled, PresaleError::AssetDisabled),
        1 => require!(state.usdc_enabled, PresaleError::AssetDisabled),
        2 => require!(state.usdt_enabled, PresaleError::AssetDisabled),
        _ => return err!(PresaleError::AssetDisabled),
    }
    let round = current_round(now, state.sale_start_ts)?;
    let price = state.prices_e9[round as usize];
    require!(price > 0, PresaleError::ZeroPrice);
    Ok((round, price))
}

fn enforce_limits(state: &PresaleState, buyer: &BuyerAccount, usd_e6: u64) -> Result<()> {
    require!(usd_e6 >= state.min_purchase_usd_e6, PresaleError::BelowMinimum);
    require!(usd_e6 <= state.max_purchase_usd_e6, PresaleError::AboveMaximum);
    let next = buyer
        .usd_contributed_e6
        .checked_add(usd_e6)
        .ok_or(PresaleError::MathOverflow)?;
    require!(next <= state.max_wallet_usd_e6, PresaleError::WalletCap);
    Ok(())
}

fn deliver_bitlab(
    state: &mut PresaleState,
    buyer: &mut BuyerAccount,
    presale: Pubkey,
    buyer_key: Pubkey,
    bitlab: u64,
    usd_e6: u64,
    now: i64,
) -> Result<()> {
    let remaining = state.allocation.saturating_sub(state.total_sold);
    require!(bitlab > 0, PresaleError::ZeroPayment);
    require!(bitlab <= remaining, PresaleError::ExceedsRemaining);
    state.total_sold = state.total_sold.checked_add(bitlab).ok_or(PresaleError::MathOverflow)?;
    if buyer.purchase_count == 0 {
        state.unique_buyers = state.unique_buyers.checked_add(1).ok_or(PresaleError::MathOverflow)?;
    } else {
        require_keys_eq!(buyer.buyer, buyer_key, PresaleError::AccountMismatch);
        require_keys_eq!(buyer.presale, presale, PresaleError::AccountMismatch);
    }
    buyer.bitlab_purchased = buyer.bitlab_purchased.checked_add(bitlab).ok_or(PresaleError::MathOverflow)?;
    buyer.usd_contributed_e6 = buyer.usd_contributed_e6.checked_add(usd_e6).ok_or(PresaleError::MathOverflow)?;
    buyer.purchase_count = buyer.purchase_count.checked_add(1).ok_or(PresaleError::MathOverflow)?;
    state.purchase_count = state.purchase_count.checked_add(1).ok_or(PresaleError::MathOverflow)?;
    if state.total_sold >= state.allocation {
        state.status = SaleStatus::SoldOut;
        state.end_reason = EndReason::SoldOut;
        state.ended_at = now;
        emit!(StatusEvent {
            status: state.status,
            reason: state.end_reason,
            timestamp: now,
        });
    }
    Ok(())
}

fn sync_status(state: &mut PresaleState, now: i64) -> Result<()> {
    if state.status == SaleStatus::Finalized
        || state.status == SaleStatus::Ended
        || state.status == SaleStatus::SoldOut
    {
        return Ok(());
    }
    if state.total_sold >= state.allocation {
        state.status = SaleStatus::SoldOut;
        state.end_reason = EndReason::SoldOut;
        state.ended_at = now;
        return Ok(());
    }
    if state.status == SaleStatus::Paused {
        return Ok(());
    }
    if state.status == SaleStatus::Active
        && now >= state.sale_start_ts.saturating_add(SALE_DURATION_SECS)
    {
        state.status = SaleStatus::Ended;
        state.end_reason = EndReason::Schedule;
        state.ended_at = now;
    }
    Ok(())
}

fn transfer_bitlab<'info>(
    token_program: &Program<'info, Token>,
    from: &Account<'info, TokenAccount>,
    to: &Account<'info, TokenAccount>,
    state: &AccountInfo<'info>,
    bump: u8,
    amount: u64,
) -> Result<()> {
    require_keys_eq!(from.mint, to.mint, PresaleError::InvalidTokenAccount);
    require_keys_eq!(from.mint, BITLAB_MINT, PresaleError::UnapprovedMint);
    transfer_signed_tokens(token_program, from, to, state, bump, amount)
}

fn transfer_signed_tokens<'info>(
    token_program: &Program<'info, Token>,
    from: &Account<'info, TokenAccount>,
    to: &Account<'info, TokenAccount>,
    state: &AccountInfo<'info>,
    bump: u8,
    amount: u64,
) -> Result<()> {
    require_keys_eq!(token_program.key(), SPL_TOKEN_PROGRAM, PresaleError::WrongTokenProgram);
    require_keys_eq!(from.owner, state.key(), PresaleError::VaultOwner);
    let seeds: &[&[u8]] = &[STATE_SEED, &[bump]];
    let binding = [seeds];
    token::transfer(
        CpiContext::new_with_signer(
            token_program.to_account_info(),
            Transfer {
                from: from.to_account_info(),
                to: to.to_account_info(),
                authority: state.clone(),
            },
            &binding,
        ),
        amount,
    )
}

fn read_u32_at(data: &[u8], off: usize) -> Result<u32> {
    let slice = data.get(off..off + 4).ok_or(PresaleError::InvalidOracle)?;
    let bytes: [u8; 4] = slice.try_into().map_err(|_| PresaleError::InvalidOracle)?;
    Ok(u32::from_le_bytes(bytes))
}
fn read_i32_at(data: &[u8], off: usize) -> Result<i32> {
    let slice = data.get(off..off + 4).ok_or(PresaleError::InvalidOracle)?;
    let bytes: [u8; 4] = slice.try_into().map_err(|_| PresaleError::InvalidOracle)?;
    Ok(i32::from_le_bytes(bytes))
}
fn read_u64_at(data: &[u8], off: usize) -> Result<u64> {
    let slice = data.get(off..off + 8).ok_or(PresaleError::InvalidOracle)?;
    let bytes: [u8; 8] = slice.try_into().map_err(|_| PresaleError::InvalidOracle)?;
    Ok(u64::from_le_bytes(bytes))
}
fn read_i64_at(data: &[u8], off: usize) -> Result<i64> {
    let slice = data.get(off..off + 8).ok_or(PresaleError::InvalidOracle)?;
    let bytes: [u8; 8] = slice.try_into().map_err(|_| PresaleError::InvalidOracle)?;
    Ok(i64::from_le_bytes(bytes))
}

fn read_pyth_price(oracle: &AccountInfo, now: i64, slot: u64, max_staleness_secs: i64) -> Result<(i64, i32, i64)> {
    require_keys_eq!(*oracle.owner, PYTH_PROGRAM, PresaleError::InvalidOracle);
    require_keys_eq!(oracle.key(), PYTH_SOL_USD, PresaleError::InvalidOracle);
    let data = oracle.try_borrow_data()?;
    require!(data.len() >= PYTH_MIN_LEN, PresaleError::InvalidOracle);
    let magic = read_u32_at(&data, PYTH_OFF_MAGIC)?;
    require!(magic == PYTH_MAGIC, PresaleError::InvalidOracle);
    let atype = read_u32_at(&data, PYTH_OFF_ATYPE)?;
    require!(atype == PYTH_PRICE_ACCOUNT_TYPE, PresaleError::InvalidOracle);
    let expo = read_i32_at(&data, PYTH_OFF_EXPO)?;
    let valid_slot = read_u64_at(&data, PYTH_OFF_VALID_SLOT)?;
    let ts = read_i64_at(&data, PYTH_OFF_TIMESTAMP)?;
    let price = read_i64_at(&data, PYTH_OFF_AGG_PRICE)?;
    let conf = read_u64_at(&data, PYTH_OFF_AGG_CONF)?;
    let status = read_u32_at(&data, PYTH_OFF_AGG_STATUS)?;
    require!(status == PYTH_STATUS_TRADING, PresaleError::InvalidOracleStatus);
    require!(price > 0, PresaleError::InvalidOracle);
    require!(expo <= 0 && expo >= -12, PresaleError::InvalidOracle);
    let abs_price = price as u64;
    let conf_bps = (conf as u128)
        .saturating_mul(10_000)
        .checked_div(abs_price.max(1) as u128)
        .ok_or(PresaleError::OracleConfidence)?;
    require!(conf_bps <= PYTH_MAX_CONF_BPS as u128, PresaleError::OracleConfidence);
    require!(now.saturating_sub(ts) <= max_staleness_secs, PresaleError::StaleOracle);
    require!(slot >= valid_slot, PresaleError::StaleOracle);
    let max_slot_lag = (max_staleness_secs as u64).saturating_mul(3).max(25);
    require!(slot.saturating_sub(valid_slot) <= max_slot_lag, PresaleError::StaleOracle);
    Ok((price, expo, ts))
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct InitializeArgs {
    pub ops_authority: Pubkey,
    pub treasury_authority: Pubkey,
    pub emergency_authority: Pubkey,
    pub treasury: Pubkey,
    pub unsold_destination: Pubkey,
    pub oracle: Pubkey,
    pub bitlab_mint: Pubkey,
    pub sale_start_ts: i64,
    pub min_purchase_usd_e6: u64,
    pub max_purchase_usd_e6: u64,
    pub max_wallet_usd_e6: u64,
    pub oracle_max_staleness_secs: i64,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init,
        payer = payer,
        space = 8 + PresaleState::INIT_SPACE,
        seeds = [STATE_SEED],
        bump
    )]
    pub state: Box<Account<'info, PresaleState>>,
    /// CHECK: empty system-owned PDA. Owner + seeds pinned. Funded to rent-exempt in handler.
    #[account(
        mut,
        seeds = [SOL_VAULT_SEED],
        bump,
        owner = system_program::ID
    )]
    pub sol_vault: UncheckedAccount<'info>,
    #[account(address = BITLAB_MINT @ PresaleError::UnapprovedMint)]
    pub bitlab_mint: Box<Account<'info, Mint>>,
    #[account(address = USDC_MINT @ PresaleError::UnapprovedMint)]
    pub usdc_mint: Box<Account<'info, Mint>>,
    #[account(address = USDT_MINT @ PresaleError::UnapprovedMint)]
    pub usdt_mint: Box<Account<'info, Mint>>,
    #[account(
        token::mint = bitlab_mint,
        token::authority = state
    )]
    pub bitlab_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        token::mint = usdc_mint,
        token::authority = state
    )]
    pub usdc_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        token::mint = usdt_mint,
        token::authority = state
    )]
    pub usdt_vault: Box<Account<'info, TokenAccount>>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct OpsOnly<'info> {
    pub ops_authority: Signer<'info>,
    #[account(mut, has_one = ops_authority @ PresaleError::Unauthorized, seeds = [STATE_SEED], bump = state.bump)]
    pub state: Box<Account<'info, PresaleState>>,
}

#[derive(Accounts)]
pub struct TreasuryOnly<'info> {
    pub treasury_authority: Signer<'info>,
    #[account(mut, has_one = treasury_authority @ PresaleError::Unauthorized, seeds = [STATE_SEED], bump = state.bump)]
    pub state: Box<Account<'info, PresaleState>>,
}

#[derive(Accounts)]
pub struct EmergencyOnly<'info> {
    pub emergency_authority: Signer<'info>,
    #[account(mut, has_one = emergency_authority @ PresaleError::Unauthorized, seeds = [STATE_SEED], bump = state.bump)]
    pub state: Box<Account<'info, PresaleState>>,
}

#[derive(Accounts)]
pub struct AcceptOps<'info> {
    pub new_authority: Signer<'info>,
    #[account(
        mut,
        seeds = [STATE_SEED],
        bump = state.bump,
        constraint = state.pending_ops_authority == new_authority.key() @ PresaleError::PendingAuthority
    )]
    pub state: Box<Account<'info, PresaleState>>,
}
#[derive(Accounts)]
pub struct AcceptTreasury<'info> {
    pub new_authority: Signer<'info>,
    #[account(
        mut,
        seeds = [STATE_SEED],
        bump = state.bump,
        constraint = state.pending_treasury_authority == new_authority.key() @ PresaleError::PendingAuthority
    )]
    pub state: Box<Account<'info, PresaleState>>,
}
#[derive(Accounts)]
pub struct AcceptEmergency<'info> {
    pub new_authority: Signer<'info>,
    #[account(
        mut,
        seeds = [STATE_SEED],
        bump = state.bump,
        constraint = state.pending_emergency_authority == new_authority.key() @ PresaleError::PendingAuthority
    )]
    pub state: Box<Account<'info, PresaleState>>,
}

#[derive(Accounts)]
pub struct BuySol<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(mut, seeds = [STATE_SEED], bump = state.bump)]
    pub state: Box<Account<'info, PresaleState>>,
    #[account(mut, address = state.treasury @ PresaleError::InvalidDestination)]
    pub treasury: SystemAccount<'info>,
    #[account(
        init_if_needed,
        payer = buyer,
        space = 8 + BuyerAccount::INIT_SPACE,
        seeds = [BUYER_SEED, state.key().as_ref(), buyer.key().as_ref()],
        bump
    )]
    pub buyer_state: Box<Account<'info, BuyerAccount>>,
    #[account(
        mut,
        token::mint = state.bitlab_mint,
        token::authority = state,
        constraint = bitlab_vault.key() == state.bitlab_vault @ PresaleError::InvalidVault
    )]
    pub bitlab_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = state.bitlab_mint,
        token::authority = buyer
    )]
    pub buyer_bitlab: Box<Account<'info, TokenAccount>>,
    /// CHECK: Official Pyth SOL/USD. Address and program owner pinned.
    #[account(
        address = PYTH_SOL_USD @ PresaleError::InvalidOracle,
        constraint = oracle.key() == state.oracle @ PresaleError::InvalidOracle,
        owner = PYTH_PROGRAM
    )]
    pub oracle: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct BuyUsdc<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(mut, seeds = [STATE_SEED], bump = state.bump)]
    pub state: Box<Account<'info, PresaleState>>,
    #[account(
        init_if_needed,
        payer = buyer,
        space = 8 + BuyerAccount::INIT_SPACE,
        seeds = [BUYER_SEED, state.key().as_ref(), buyer.key().as_ref()],
        bump
    )]
    pub buyer_state: Box<Account<'info, BuyerAccount>>,
    #[account(
        mut,
        token::mint = state.bitlab_mint,
        token::authority = state,
        constraint = bitlab_vault.key() == state.bitlab_vault @ PresaleError::InvalidVault
    )]
    pub bitlab_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = state.bitlab_mint,
        token::authority = buyer
    )]
    pub buyer_bitlab: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = state.usdc_mint,
        token::authority = buyer
    )]
    pub buyer_usdc: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = state.usdc_mint,
        token::authority = state.treasury
    )]
    pub treasury_usdc: Box<Account<'info, TokenAccount>>,
    #[account(address = state.usdc_mint @ PresaleError::UnapprovedMint)]
    pub usdc_mint: Box<Account<'info, Mint>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct BuyUsdt<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(mut, seeds = [STATE_SEED], bump = state.bump)]
    pub state: Box<Account<'info, PresaleState>>,
    #[account(
        init_if_needed,
        payer = buyer,
        space = 8 + BuyerAccount::INIT_SPACE,
        seeds = [BUYER_SEED, state.key().as_ref(), buyer.key().as_ref()],
        bump
    )]
    pub buyer_state: Box<Account<'info, BuyerAccount>>,
    #[account(
        mut,
        token::mint = state.bitlab_mint,
        token::authority = state,
        constraint = bitlab_vault.key() == state.bitlab_vault @ PresaleError::InvalidVault
    )]
    pub bitlab_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = state.bitlab_mint,
        token::authority = buyer
    )]
    pub buyer_bitlab: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = state.usdt_mint,
        token::authority = buyer
    )]
    pub buyer_usdt: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = state.usdt_mint,
        token::authority = state.treasury
    )]
    pub treasury_usdt: Box<Account<'info, TokenAccount>>,
    #[account(address = state.usdt_mint @ PresaleError::UnapprovedMint)]
    pub usdt_mint: Box<Account<'info, Mint>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawSol<'info> {
    pub treasury_authority: Signer<'info>,
    #[account(mut, has_one = treasury_authority @ PresaleError::Unauthorized, seeds = [STATE_SEED], bump = state.bump)]
    pub state: Box<Account<'info, PresaleState>>,
    /// CHECK: SOL vault PDA, system-owned, canonical bump.
    #[account(
        mut,
        seeds = [SOL_VAULT_SEED],
        bump = state.sol_vault_bump,
        owner = system_program::ID,
        constraint = sol_vault.key() == state.sol_vault @ PresaleError::InvalidVault
    )]
    pub sol_vault: UncheckedAccount<'info>,
    #[account(mut, address = state.treasury @ PresaleError::InvalidDestination)]
    pub destination: SystemAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct EmergencyWithdrawSol<'info> {
    pub emergency_authority: Signer<'info>,
    #[account(mut, has_one = emergency_authority @ PresaleError::Unauthorized, seeds = [STATE_SEED], bump = state.bump)]
    pub state: Box<Account<'info, PresaleState>>,
    /// CHECK: SOL vault PDA, system-owned, canonical bump.
    #[account(
        mut,
        seeds = [SOL_VAULT_SEED],
        bump = state.sol_vault_bump,
        owner = system_program::ID,
        constraint = sol_vault.key() == state.sol_vault @ PresaleError::InvalidVault
    )]
    pub sol_vault: UncheckedAccount<'info>,
    #[account(mut, address = state.treasury @ PresaleError::InvalidDestination)]
    pub destination: SystemAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawUsdc<'info> {
    pub treasury_authority: Signer<'info>,
    #[account(mut, has_one = treasury_authority @ PresaleError::Unauthorized, seeds = [STATE_SEED], bump = state.bump)]
    pub state: Box<Account<'info, PresaleState>>,
    #[account(
        mut,
        token::mint = state.usdc_mint,
        token::authority = state,
        constraint = usdc_vault.key() == state.usdc_vault @ PresaleError::InvalidVault
    )]
    pub usdc_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = state.usdc_mint,
        token::authority = state.treasury
    )]
    pub destination: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct WithdrawUsdt<'info> {
    pub treasury_authority: Signer<'info>,
    #[account(mut, has_one = treasury_authority @ PresaleError::Unauthorized, seeds = [STATE_SEED], bump = state.bump)]
    pub state: Box<Account<'info, PresaleState>>,
    #[account(
        mut,
        token::mint = state.usdt_mint,
        token::authority = state,
        constraint = usdt_vault.key() == state.usdt_vault @ PresaleError::InvalidVault
    )]
    pub usdt_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = state.usdt_mint,
        token::authority = state.treasury
    )]
    pub destination: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct EmergencyWithdrawUsdc<'info> {
    pub emergency_authority: Signer<'info>,
    #[account(mut, has_one = emergency_authority @ PresaleError::Unauthorized, seeds = [STATE_SEED], bump = state.bump)]
    pub state: Box<Account<'info, PresaleState>>,
    #[account(
        mut,
        token::mint = state.usdc_mint,
        token::authority = state,
        constraint = usdc_vault.key() == state.usdc_vault @ PresaleError::InvalidVault
    )]
    pub usdc_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = state.usdc_mint,
        token::authority = state.treasury
    )]
    pub destination: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct EmergencyWithdrawUsdt<'info> {
    pub emergency_authority: Signer<'info>,
    #[account(mut, has_one = emergency_authority @ PresaleError::Unauthorized, seeds = [STATE_SEED], bump = state.bump)]
    pub state: Box<Account<'info, PresaleState>>,
    #[account(
        mut,
        token::mint = state.usdt_mint,
        token::authority = state,
        constraint = usdt_vault.key() == state.usdt_vault @ PresaleError::InvalidVault
    )]
    pub usdt_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = state.usdt_mint,
        token::authority = state.treasury
    )]
    pub destination: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct FinalizeSale<'info> {
    pub ops_authority: Signer<'info>,
    #[account(mut, has_one = ops_authority @ PresaleError::Unauthorized, seeds = [STATE_SEED], bump = state.bump)]
    pub state: Box<Account<'info, PresaleState>>,
    #[account(
        mut,
        token::mint = state.bitlab_mint,
        token::authority = state,
        constraint = bitlab_vault.key() == state.bitlab_vault @ PresaleError::InvalidVault
    )]
    pub bitlab_vault: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        token::mint = state.bitlab_mint,
        token::authority = state.unsold_destination
    )]
    pub unsold_destination: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

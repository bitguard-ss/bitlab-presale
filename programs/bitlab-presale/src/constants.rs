use anchor_lang::prelude::*;

pub const STATE_SEED: &[u8] = b"bitlab-presale";
pub const SOL_VAULT_SEED: &[u8] = b"bitlab-sol-vault";
pub const BUYER_SEED: &[u8] = b"bitlab-buyer";

pub const ROUND_COUNT: u8 = 10;
pub const ROUND_DURATION_SECS: i64 = 7 * 24 * 60 * 60;
pub const SALE_DURATION_SECS: i64 = ROUND_DURATION_SECS * ROUND_COUNT as i64;

/// $0.001 * 1e9, then +10% each round (integer, matching the public schedule).
pub const ROUND_PRICES_E9: [u64; 10] = [
    1_000_000,
    1_100_000,
    1_210_000,
    1_331_000,
    1_464_100,
    1_610_510,
    1_771_561,
    1_948_717,
    2_143_589,
    2_357_948,
];

pub const PRICE_SCALE_E9: u128 = 1_000_000_000;
pub const PRESALE_ALLOCATION: u64 = 7_500_000_000_000_000; // 7.5B * 1e6
pub const BITLAB_DECIMALS: u8 = 6;
pub const STABLE_DECIMALS: u8 = 6;
pub const SOL_DECIMALS: i32 = 9;

pub const DEFAULT_MIN_PURCHASE_USD_E6: u64 = 10_000_000;
pub const DEFAULT_MAX_PURCHASE_USD_E6: u64 = 100_000_000_000;
pub const DEFAULT_MAX_WALLET_USD_E6: u64 = 500_000_000_000;
pub const DEFAULT_ORACLE_STALENESS_SECS: i64 = 60;
/// Lower bound prevents a 1-second window that DoS-es SOL buys on clock skew.
pub const ORACLE_STALENESS_MIN_SECS: i64 = 15;
/// Upper bound prevents "any stale price is fine" misconfiguration.
pub const ORACLE_STALENESS_MAX_SECS: i64 = 300;

/// Official mainnet mints / programs. Initialize rejects anything else.
pub const BITLAB_MINT: Pubkey = pubkey!("CCVYpFakBd4kkwn7G4rvbpaiB5pGJwUpiNwNYrkC5ygQ");
pub const USDC_MINT: Pubkey = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
pub const USDT_MINT: Pubkey = pubkey!("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB");
pub const PYTH_PROGRAM: Pubkey = pubkey!("FsJ3A3u2vn5cTVofAjvy6y5kwAB3Aki3PsVaKEP8s4ir");
pub const PYTH_SOL_USD: Pubkey = pubkey!("H6ARHf6YXhGYeQfUzQNGk6rDNnLBQKrenN712K4AQJEG");
pub const SPL_TOKEN_PROGRAM: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

pub const PYTH_MAGIC: u32 = 0xa1b2c3d4;
pub const PYTH_PRICE_ACCOUNT_TYPE: u32 = 3;
pub const PYTH_STATUS_TRADING: u32 = 1;
/// Reject SOL quotes when confidence is worse than 10% of price.
pub const PYTH_MAX_CONF_BPS: u64 = 1_000;

/// Layout matches pyth-client-js `parsePriceData` / C `pc_price_t` on Solana mainnet.
/// Do not use prev_price (offset 184) — that is the previous aggregate, not the live one.
pub const PYTH_OFF_MAGIC: usize = 0;
pub const PYTH_OFF_ATYPE: usize = 8;
pub const PYTH_OFF_EXPO: usize = 20;
pub const PYTH_OFF_VALID_SLOT: usize = 40;
pub const PYTH_OFF_TIMESTAMP: usize = 96;
pub const PYTH_OFF_AGG_PRICE: usize = 208;
pub const PYTH_OFF_AGG_CONF: usize = 216;
pub const PYTH_OFF_AGG_STATUS: usize = 224;
pub const PYTH_MIN_LEN: usize = 240;

pub fn current_round(now: i64, start: i64) -> Result<u8> {
    require!(now >= start, crate::error::PresaleError::NotStarted);
    let elapsed = now
        .checked_sub(start)
        .ok_or(crate::error::PresaleError::MathOverflow)?;
    require!(elapsed < SALE_DURATION_SECS, crate::error::PresaleError::ScheduleEnded);
    Ok((elapsed / ROUND_DURATION_SECS) as u8)
}

pub fn bitlab_from_usd_e6(usd_e6: u64, price_e9: u64) -> Result<u64> {
    require!(price_e9 > 0, crate::error::PresaleError::ZeroPrice);
    let raw = (usd_e6 as u128)
        .checked_mul(PRICE_SCALE_E9)
        .ok_or(crate::error::PresaleError::MathOverflow)?
        .checked_div(price_e9 as u128)
        .ok_or(crate::error::PresaleError::MathOverflow)?;
    u64::try_from(raw).map_err(|_| crate::error::PresaleError::MathOverflow.into())
}

/// usd_e6 = lamports * pyth_price / 10^(sol_decimals - stable_decimals - expo)
/// expo is typically -8 → divide by 10^11.
pub fn usd_e6_from_sol(lamports: u64, pyth_price: i64, pyth_expo: i32) -> Result<u64> {
    require!(pyth_price > 0, crate::error::PresaleError::InvalidOracle);
    require!(pyth_expo <= 0 && pyth_expo >= -12, crate::error::PresaleError::InvalidOracle);
    let exp = pyth_expo + (STABLE_DECIMALS as i32) - SOL_DECIMALS;
    let price = pyth_price as u128;
    let lamports = lamports as u128;
    let usd = if exp >= 0 {
        lamports
            .checked_mul(price)
            .ok_or(crate::error::PresaleError::MathOverflow)?
            .checked_mul(pow10(exp as u32)?)
            .ok_or(crate::error::PresaleError::MathOverflow)?
    } else {
        lamports
            .checked_mul(price)
            .ok_or(crate::error::PresaleError::MathOverflow)?
            .checked_div(pow10((-exp) as u32)?)
            .ok_or(crate::error::PresaleError::MathOverflow)?
    };
    require!(usd > 0, crate::error::PresaleError::ZeroPayment);
    u64::try_from(usd).map_err(|_| crate::error::PresaleError::MathOverflow.into())
}

fn pow10(exp: u32) -> Result<u128> {
    require!(exp <= 12, crate::error::PresaleError::InvalidOracle);
    Ok(10u128.pow(exp))
}

pub fn assert_pubkey_set(key: &Pubkey) -> Result<()> {
    require!(*key != Pubkey::default(), crate::error::PresaleError::InvalidAuthority);
    Ok(())
}

pub fn assert_limit_invariant(min_usd: u64, max_usd: u64, max_wallet: u64) -> Result<()> {
    require!(min_usd >= 1, crate::error::PresaleError::LimitInvariant);
    require!(max_usd >= min_usd, crate::error::PresaleError::LimitInvariant);
    require!(max_wallet >= max_usd, crate::error::PresaleError::LimitInvariant);
    Ok(())
}

pub fn assert_staleness_bounds(secs: i64) -> Result<i64> {
    require!(
        secs >= ORACLE_STALENESS_MIN_SECS && secs <= ORACLE_STALENESS_MAX_SECS,
        crate::error::PresaleError::OracleStalenessBounds
    );
    Ok(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_dollars_at_round_one_is_ten_thousand_bitlab() {
        let raw = bitlab_from_usd_e6(10_000_000, 1_000_000).unwrap();
        assert_eq!(raw, 10_000_000_000); // 10,000 * 1e6
    }

    #[test]
    fn one_sol_at_150_usd() {
        // price 150 with expo -8 → 150 * 10^8
        let usd = usd_e6_from_sol(1_000_000_000, 15_000_000_000, -8).unwrap();
        assert_eq!(usd, 150_000_000);
    }

    #[test]
    fn rejects_zero_and_negative_oracle_price() {
        assert!(usd_e6_from_sol(1_000_000_000, 0, -8).is_err());
        assert!(usd_e6_from_sol(1_000_000_000, -1, -8).is_err());
    }

    #[test]
    fn rejects_wild_expo() {
        assert!(usd_e6_from_sol(1_000_000_000, 15_000_000_000, -13).is_err());
        assert!(usd_e6_from_sol(1_000_000_000, 15_000_000_000, 1).is_err());
    }

    #[test]
    fn round_window() {
        let start = 1_800_000_000;
        assert_eq!(current_round(start, start).unwrap(), 0);
        assert_eq!(current_round(start + ROUND_DURATION_SECS - 1, start).unwrap(), 0);
        assert_eq!(current_round(start + ROUND_DURATION_SECS, start).unwrap(), 1);
        assert_eq!(current_round(start + SALE_DURATION_SECS - 1, start).unwrap(), 9);
        assert!(current_round(start + SALE_DURATION_SECS, start).is_err());
        assert!(current_round(start - 1, start).is_err());
    }

    #[test]
    fn limit_invariant() {
        assert!(assert_limit_invariant(1, 1, 1).is_ok());
        assert!(assert_limit_invariant(0, 1, 1).is_err());
        assert!(assert_limit_invariant(2, 1, 2).is_err());
        assert!(assert_limit_invariant(1, 5, 4).is_err());
    }

    #[test]
    fn pyth_offsets_match_pyth_client_js() {
        assert_eq!(PYTH_OFF_AGG_PRICE, 208);
        assert_eq!(PYTH_OFF_AGG_CONF, 216);
        assert_eq!(PYTH_OFF_AGG_STATUS, 224);
        assert_eq!(PYTH_OFF_TIMESTAMP, 96);
        assert_eq!(PYTH_OFF_VALID_SLOT, 40);
    }
}

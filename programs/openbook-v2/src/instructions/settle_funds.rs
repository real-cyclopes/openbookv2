use anchor_lang::prelude::*;
use anchor_spl::token::{self, Transfer};
use fixed::types::I80F48;

use crate::accounts_ix::*;
use crate::state::*;

pub fn settle_funds(ctx: Context<SettleFunds>) -> Result<()> {
    let mut open_orders_account = ctx.accounts.open_orders_account.load_full_mut()?;
    let market = ctx.accounts.market.load_mut()?;
    let mut position = &mut open_orders_account.fixed_mut().position;

    let seeds = [
        b"Market".as_ref(),
        &market.market_index.to_le_bytes(),
        &[market.bump],
    ];
    let signer = &[&seeds[..]];

    let base_amount_native =
        I80F48::from(market.base_lot_size) * I80F48::from(position.base_free_lots);
    position.base_free_lots = 0;

    let quote_amount_native =
        I80F48::from(market.quote_lot_size) * I80F48::from(position.quote_free_lots);
    position.quote_free_lots = 0;

    if base_amount_native > 0 {
        let cpi_context = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.base_vault.to_account_info(),
                to: ctx.accounts.payer_base.to_account_info(),
                authority: ctx.accounts.market.to_account_info(),
            },
        );
        token::transfer(cpi_context.with_signer(signer), base_amount_native.to_num())?;
    }

    if quote_amount_native > 0 {
        let cpi_context = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.quote_vault.to_account_info(),
                to: ctx.accounts.payer_quote.to_account_info(),
                authority: ctx.accounts.market.to_account_info(),
            },
        );
        token::transfer(
            cpi_context.with_signer(signer),
            quote_amount_native.to_num(),
        )?;
    }

    // let fee_amount = market.fees_accrued;
    // if ctx.remaining_accounts.len() > 0 && fee_amount >0 {
    //     let referrer = &ctx.remaining_accounts[0].to_account_info();
    //         let cpi_context = CpiContext::new(
    //             ctx.accounts.token_program.to_account_info(),
    //             Transfer {
    //                 from: ctx.accounts.quote_vault.to_account_info(),
    //                 to: referrer,
    //                 authority: ctx.accounts.market.to_account_info(),
    //             },
    //         );
    //         token::transfer(
    //             cpi_context.with_signer(signer),
    //             fee_amount.to_num(),
    //         )?;

    //     market.fees_settled += market.fees_accrued;
    //     market.fee_acrued -=fee_amount;
    // }

    Ok(())
}

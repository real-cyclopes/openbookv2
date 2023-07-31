use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

#[derive(Accounts)]
pub struct SettleFunds<'info> {
    pub owner: Signer<'info>,
    #[account(
        mut,
        has_one = owner,
        has_one = market,
    )]
    pub open_orders_account: AccountLoader<'info, OpenOrdersAccount>,

    #[account(
        mut,
        has_one = base_vault,
        has_one = quote_vault,
        has_one = market_authority,
    )]
    pub market: AccountLoader<'info, Market>,
    /// CHECK: checked on has_one in market
    pub market_authority: UncheckedAccount<'info>,
    #[account(mut)]
    pub base_vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub quote_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        token::mint = base_vault.mint
    )]
    pub token_base_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        token::mint = quote_vault.mint
    )]
    pub token_quote_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        token::mint = quote_vault.mint
    )]
    pub referrer: Option<Box<Account<'info, TokenAccount>>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

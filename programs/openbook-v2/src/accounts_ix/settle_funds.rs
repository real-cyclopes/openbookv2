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
        has_one = market_base_vault,
        has_one = market_quote_vault,
        has_one = market_authority,
    )]
    pub market: AccountLoader<'info, Market>,
    /// CHECK: checked on has_one in market
    pub market_authority: UncheckedAccount<'info>,
    #[account(mut)]
    pub market_base_vault: Account<'info, TokenAccount>,
    #[account(mut)]
    pub market_quote_vault: Account<'info, TokenAccount>,
    #[account(
        mut,
        token::mint = market_base_vault.mint
    )]
    pub user_base_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        token::mint = market_quote_vault.mint
    )]
    pub user_quote_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        token::mint = market_quote_vault.mint
    )]
    pub referrer: Option<Box<Account<'info, TokenAccount>>>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

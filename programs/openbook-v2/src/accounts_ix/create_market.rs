use crate::state::*;
use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{Mint, Token, TokenAccount},
};

#[event_cpi]
#[derive(Accounts)]
pub struct CreateMarket<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + std::mem::size_of::<Market>(),
    )]
    pub market: AccountLoader<'info, Market>,
    #[account(
        seeds = [b"Market".as_ref(), market.key().to_bytes().as_ref()],
        bump,
    )]
    /// CHECK:
    pub market_authority: UncheckedAccount<'info>,

    /// Accounts are initialized by client,
    /// anchor discriminator is set first when ix exits,
    #[account(zero)]
    pub bids: AccountLoader<'info, BookSide>,
    #[account(zero)]
    pub asks: AccountLoader<'info, BookSide>,
    #[account(zero)]
    pub event_heap: AccountLoader<'info, EventHeap>,

    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        init,
        payer = payer,
        associated_token::mint = base_mint,
        associated_token::authority = market_authority,
    )]
    pub market_base_vault: Account<'info, TokenAccount>,
    #[account(
        init,
        payer = payer,
        associated_token::mint = quote_mint,
        associated_token::authority = market_authority,
    )]
    pub market_quote_vault: Account<'info, TokenAccount>,

    #[account(constraint = base_mint.key() != quote_mint.key())]
    pub base_mint: Box<Account<'info, Mint>>,
    pub quote_mint: Box<Account<'info, Mint>>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    /// CHECK: The oracle can be one of several different account types
    pub oracle_a: Option<UncheckedAccount<'info>>,
    /// CHECK: The oracle can be one of several different account types
    pub oracle_b: Option<UncheckedAccount<'info>>,

    /// CHECK:
    pub collect_fee_admin: UncheckedAccount<'info>,
    /// CHECK:
    pub open_orders_admin: Option<UncheckedAccount<'info>>,
    /// CHECK:
    pub consume_events_admin: Option<UncheckedAccount<'info>>,
    /// CHECK:
    pub close_market_admin: Option<UncheckedAccount<'info>>,
}

use crate::state::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct PruneOrders<'info> {
    pub close_market_admin: Signer<'info>,
    // owner is not checked, only close_market_admin
    pub owner: UncheckedAccount<'info>,
    #[account(
        mut,
        has_one = market
    )]
    pub open_orders_account: AccountLoader<'info, OpenOrdersAccountFixed>,
    #[account(
        has_one = bids,
        has_one = asks,
    )]
    pub market: AccountLoader<'info, Market>,
    #[account(mut)]
    pub bids: AccountLoader<'info, BookSide>,
    #[account(mut)]
    pub asks: AccountLoader<'info, BookSide>,
}

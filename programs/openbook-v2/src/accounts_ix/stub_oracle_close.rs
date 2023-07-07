use anchor_lang::prelude::*;
use anchor_spl::token::Token;

use crate::state::*;

#[derive(Accounts)]
pub struct StubOracleClose<'info> {
    pub admin: Signer<'info>,
    #[account(
        mut,
        close = sol_destination
    )]
    pub oracle: AccountLoader<'info, StubOracle>,
    #[account(mut)]
    /// CHECK: target for account rent needs no checks
    pub sol_destination: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token>,
}

use anchor_lang::AccountDeserialize;
use anchor_lang::__private::bytemuck::Zeroable;
use anyhow::Result;
use fixed::types::I80F48;
use openbook_v2::accounts_zerocopy::{AccountReader, KeyedAccountReader};
use openbook_v2::state::{BookSide, OrderParams};
use openbook_v2::state::{
    EventQueue, Market, Order, Orderbook, SelfTradeBehavior::DecrementTake, Side,
};

use rust_decimal::Decimal;
use solana_sdk::sysvar::clock::Clock;
/// An abstraction in order to share reserve mints and necessary data
use solana_sdk::{account::Account, pubkey::Pubkey};
use std::cell::RefCell;
use std::collections::HashMap;
use std::str;
use std::time::{SystemTime, UNIX_EPOCH};

// TODO Adjust this number after doing some calculations
const MAXIUM_TAKEN_ORDERS: u8 = 8;

#[derive(Clone)]
pub struct KeyedAccount {
    pub key: Pubkey,
    pub account: Account,
}

pub struct QuoteParams {
    pub max_base_lots: i64,
    pub max_quote_lots_including_fees: i64,
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Quote {
    pub not_enough_liquidity: bool,
    pub min_in_amount: Option<u64>,
    pub min_out_amount: Option<u64>,
    pub in_amount: u64,
    pub out_amount: u64,
    pub fee_amount: u64,
    pub fee_mint: Pubkey,
    pub fee_pct: Decimal,
    pub price_impact_pct: Decimal,
}

pub trait Amm {
    // Label of your Amm
    fn label(&self) -> String;
    // Identifier for your amm, should be your pool id
    fn key(&self) -> Pubkey;
    // The token mints that the pool support for swapping
    fn get_reserve_mints(&self) -> Vec<Pubkey>;
    // Related accounts to get the quote for swapping and creating ix
    fn get_accounts_to_update(&self) -> Vec<Pubkey>;
    // Picks data necessary to update it's internal state
    fn update(&mut self, accounts_map: &HashMap<Pubkey, Vec<u8>>) -> Result<()>;
    // Compute the quote from internal state
    fn quote(&self, quote_params: &QuoteParams) -> Result<Quote>;
}

pub struct OpenBookMarket {
    market: Market,
    event_queue: EventQueue,
    bids: BookSide,
    asks: BookSide,
    key: Pubkey,
    label: String,
    related_accounts: [Pubkey; 6],
    reserve_mints: [Pubkey; 2],
    reserves: [u128; 2],
    program_id: Pubkey,
    oracle_price: I80F48,
}

impl OpenBookMarket {
    pub fn from_keyed_account(keyed_account: &KeyedAccount) -> Result<Self> {
        let market = Market::try_deserialize(&mut keyed_account.account.data.as_slice())?;

        Ok(OpenBookMarket {
            market,
            key: keyed_account.key,
            label: str::from_utf8(&market.name).unwrap().to_string(),
            related_accounts: [
                market.bids,
                market.asks,
                market.event_queue,
                market.oracle,
                market.base_vault,
                market.quote_vault,
            ],
            reserve_mints: [market.base_mint, market.quote_mint],
            reserves: [0, 0],
            program_id: openbook_v2::ID,
            event_queue: EventQueue::zeroed(),
            bids: BookSide::zeroed(),
            asks: BookSide::zeroed(),
            oracle_price: I80F48::ZERO,
        })
    }
}

impl Amm for OpenBookMarket {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn key(&self) -> Pubkey {
        self.key
    }

    fn get_reserve_mints(&self) -> Vec<Pubkey> {
        self.reserve_mints.to_vec()
    }

    fn get_accounts_to_update(&self) -> Vec<Pubkey> {
        self.related_accounts.to_vec()
    }

    fn update(&mut self, accounts_map: &HashMap<Pubkey, Vec<u8>>) -> Result<()> {
        let bids_data: &Vec<u8> = accounts_map.get(&self.market.bids).unwrap();
        self.bids = BookSide::try_deserialize(&mut bids_data.as_slice()).unwrap();

        let asks_data = accounts_map.get(&self.market.asks).unwrap();
        self.asks = BookSide::try_deserialize(&mut asks_data.as_slice()).unwrap();

        let event_queue_data = accounts_map.get(&self.market.event_queue).unwrap();
        self.event_queue = EventQueue::try_deserialize(&mut event_queue_data.as_slice()).unwrap();

        let oracle_data = accounts_map.get(&self.market.oracle).unwrap();
        let oracle_acc = &AccountOracle {
            data: oracle_data,
            key: self.market.oracle,
        };

        let timestamp = Clock::default().unix_timestamp;
        self.oracle_price = self.market.oracle_price(oracle_acc, timestamp as u64)?;

        Ok(())
    }

    fn quote(&self, quote_params: &QuoteParams) -> Result<Quote> {
        let side = if quote_params.input_mint == self.market.quote_mint {
            Side::Bid
        } else {
            Side::Ask
        };

        let order = &Order {
            side,
            max_base_lots: quote_params.max_base_lots,
            max_quote_lots_including_fees: quote_params.max_quote_lots_including_fees,
            client_order_id: 0,
            time_in_force: 0,
            self_trade_behavior: DecrementTake,
            params: OrderParams::Market,
        };

        let bids_ref = RefCell::new(self.bids);
        let asks_ref = RefCell::new(self.asks);
        let book = Orderbook {
            bids: bids_ref.borrow_mut(),
            asks: asks_ref.borrow_mut(),
        };

        let timestamp = Clock::default().unix_timestamp;

        let order_amounts: Amounts = iterate_book(
            book,
            order,
            &self.market,
            self.oracle_price,
            timestamp as u64,
        )?;
        let (in_amount, out_amount) = match side {
            Side::Bid => (
                order_amounts.total_quote_taken_native,
                order_amounts.total_base_taken_native,
            ),
            Side::Ask => (
                order_amounts.total_base_taken_native,
                order_amounts.total_quote_taken_native,
            ),
        };

        Ok(Quote {
            in_amount,
            out_amount,
            fee_mint: self.market.quote_mint,
            fee_amount: order_amounts.fee,
            price_impact_pct: order_amounts.price_impact.into(),
            ..Quote::default()
        })
    }
}

pub struct Amounts {
    pub total_base_taken_native: u64,
    pub total_quote_taken_native: u64,
    pub fee: u64,
    pub price_impact: i64,
    pub not_enough_liquidity: bool,
}

fn iterate_book(
    book: Orderbook,
    order: &Order,
    market: &Market,
    oracle_price: I80F48,
    now_ts: u64,
) -> Result<Amounts> {
    let side = order.side;
    let mut limit = MAXIUM_TAKEN_ORDERS;

    let other_side = side.invert_side();
    let oracle_price_lots = market.native_price_to_lot(oracle_price)?;
    let (price_lots, _) = order.price(now_ts, oracle_price_lots, &book)?;

    let order_max_quote_lots = match side {
        Side::Bid => market.subtract_taker_fees(order.max_quote_lots_including_fees),
        Side::Ask => order.max_quote_lots_including_fees,
    };

    let mut remaining_base_lots = order.max_base_lots;
    let mut remaining_quote_lots = order_max_quote_lots;

    let mut first_price = 0_i64;
    let mut last_price = 0_i64;

    let opposing_bookside = book.bookside(other_side);
    for (index, best_opposing) in opposing_bookside
        .iter_valid(now_ts, oracle_price_lots)
        .enumerate()
    {
        if remaining_base_lots == 0 || remaining_quote_lots == 0 {
            break;
        }

        if index == 0 {
            first_price = best_opposing.price_lots;
        } else {
            last_price = best_opposing.price_lots;
        }

        let best_opposing_price = best_opposing.price_lots;

        if !side.is_price_within_limit(best_opposing_price, price_lots) || limit == 0 {
            break;
        }

        let max_match_by_quote = remaining_quote_lots / best_opposing_price;
        if max_match_by_quote == 0 {
            break;
        }

        let match_base_lots = remaining_base_lots
            .min(best_opposing.node.quantity)
            .min(max_match_by_quote);
        let match_quote_lots = match_base_lots * best_opposing_price;

        remaining_base_lots -= match_base_lots;
        remaining_quote_lots -= match_quote_lots;
        assert!(remaining_quote_lots >= 0);

        limit -= 1;
    }

    let total_quote_lots_taken = order_max_quote_lots - remaining_quote_lots;
    let total_base_lots_taken = order.max_base_lots - remaining_base_lots;

    assert!(total_quote_lots_taken >= 0);
    assert!(total_base_lots_taken >= 0);

    let mut total_base_taken_native = (total_base_lots_taken * market.base_lot_size) as u64;

    let mut total_quote_taken_native = (total_quote_lots_taken * market.quote_lot_size) as u64;

    let mut taker_fees = 0_u64;
    let mut not_enough_liquidity = false;
    if total_quote_lots_taken > 0 || total_base_lots_taken > 0 {
        taker_fees = market.taker_fees_ceil(total_quote_taken_native);

        match side {
            Side::Bid => {
                total_quote_taken_native += taker_fees;
            }
            Side::Ask => {
                total_quote_taken_native -= taker_fees;
            }
        };
    } else {
        not_enough_liquidity = true;
    }

    Ok(Amounts {
        total_base_taken_native,
        total_quote_taken_native,
        fee: taker_fees,
        price_impact: (first_price - last_price).abs(),
        not_enough_liquidity,
    })
}

pub struct AccountOracle<'a> {
    data: &'a Vec<u8>,
    key: Pubkey,
}

impl AccountReader for AccountOracle<'_> {
    fn owner(&self) -> &Pubkey {
        return &self.key;
    }

    fn data(&self) -> &[u8] {
        return &self.data;
    }
}

impl KeyedAccountReader for AccountOracle<'_> {
    fn key(&self) -> &Pubkey {
        return &self.key;
    }
}

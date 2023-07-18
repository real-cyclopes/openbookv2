use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anchor_client::Cluster;

use anchor_lang::prelude::System;
use anchor_lang::{AccountDeserialize, Id};
use anchor_spl::token::Token;

use itertools::Itertools;

use openbook_v2::state::{MarketIndex, OpenOrdersAccount, PlaceOrderType, SelfTradeBehavior, Side};

use solana_client::nonblocking::rpc_client::RpcClient as RpcClientAsync;
use solana_client::rpc_config::RpcSendTransactionConfig;
use solana_sdk::address_lookup_table_account::AddressLookupTableAccount;
use solana_sdk::commitment_config::CommitmentLevel;
use solana_sdk::hash::Hash;
use solana_sdk::signer::keypair;
use solana_sdk::transaction::TransactionError;

use crate::account_fetcher::*;
use crate::context::OpenBookContext;
use crate::gpa::{fetch_anchor_account, fetch_openbook_accounts};

use anyhow::Context;
use solana_sdk::account::ReadableAccount;
use solana_sdk::instruction::Instruction;
use solana_sdk::signature::{Keypair, Signature};
use solana_sdk::{commitment_config::CommitmentConfig, pubkey::Pubkey, signer::Signer};

// very close to anchor_client::Client, which unfortunately has no accessors or Clone
#[derive(Clone, Debug)]
pub struct Client {
    pub cluster: Cluster,
    pub fee_payer: Arc<Keypair>,
    pub commitment: CommitmentConfig,
    pub timeout: Option<Duration>,
    pub transaction_builder_config: TransactionBuilderConfig,
    pub rpc_send_transaction_config: RpcSendTransactionConfig,
}

impl Client {
    pub fn new(
        cluster: Cluster,
        commitment: CommitmentConfig,
        fee_payer: Arc<Keypair>,
        timeout: Option<Duration>,
        transaction_builder_config: TransactionBuilderConfig,
    ) -> Self {
        Self {
            cluster,
            fee_payer,
            commitment,
            timeout,
            transaction_builder_config,
            rpc_send_transaction_config: RpcSendTransactionConfig {
                preflight_commitment: Some(CommitmentLevel::Processed),
                ..Default::default()
            },
        }
    }

    pub fn rpc_async(&self) -> RpcClientAsync {
        let url = self.cluster.url().to_string();
        if let Some(timeout) = self.timeout.as_ref() {
            RpcClientAsync::new_with_timeout_and_commitment(url, *timeout, self.commitment)
        } else {
            RpcClientAsync::new_with_commitment(url, self.commitment)
        }
    }

    pub async fn rpc_anchor_account<T: AccountDeserialize>(
        &self,
        address: &Pubkey,
    ) -> anyhow::Result<T> {
        fetch_anchor_account(&self.rpc_async(), address).await
    }
}

// todo: might want to integrate geyser, websockets, or simple http polling for keeping data fresh
pub struct OpenBookClient {
    pub client: Client,

    // todo: possibly this object should have cache-functions, so there can be one getMultipleAccounts
    // call to refresh -- if it's backed by websockets, these could just do nothing
    pub account_fetcher: Arc<dyn AccountFetcher>,

    pub owner: Arc<Keypair>,
    pub open_orders_account: Pubkey,

    pub context: OpenBookContext,

    pub http_client: reqwest::Client,
}

// TODO: add retry framework for sending tx and rpc calls
// 1/ this works right now, but I think mid-term the OpenBookClient will want to interact with multiple openorders accounts
// -- then we should probably specify accounts by owner+account_num / or pubkey
// 2/ pubkey, can be both owned, but also delegated accounts

impl OpenBookClient {
    pub async fn find_accounts(
        client: &Client,
        owner: &Keypair,
    ) -> anyhow::Result<Vec<(Pubkey, OpenOrdersAccount)>> {
        fetch_openbook_accounts(&client.rpc_async(), openbook_v2::ID, owner.pubkey()).await
    }

    pub async fn find_or_create_account(
        client: &Client,
        owner: &Keypair,
        payer: &Keypair, // pays the SOL for the new account
        market: Pubkey,
        openbook_account_name: &str,
    ) -> anyhow::Result<Pubkey> {
        let rpc = client.rpc_async();
        let program = openbook_v2::ID;

        let mut openbook_account_tuples =
            fetch_openbook_accounts(&rpc, program, owner.pubkey()).await?;
        let openbook_account_opt = openbook_account_tuples
            .iter()
            .find(|(_, account)| account.name() == openbook_account_name);
        if openbook_account_opt.is_none() {
            openbook_account_tuples
                .sort_by(|a, b| a.1.account_num.partial_cmp(&b.1.account_num).unwrap());
            let account_num = match openbook_account_tuples.last() {
                Some(tuple) => tuple.1.account_num + 1,
                None => 0u32,
            };
            Self::init_open_orders(client, market, owner, payer, None, account_num)
                .await
                .context("Failed to create account...")?;
        }
        let openbook_account_tuples =
            fetch_openbook_accounts(&rpc, program, owner.pubkey()).await?;
        let index = openbook_account_tuples
            .iter()
            .position(|tuple| tuple.1.name() == openbook_account_name)
            .unwrap();
        Ok(openbook_account_tuples[index].0)
    }

    pub async fn init_open_orders(
        client: &Client,
        market: Pubkey,
        owner: &Keypair,
        payer: &Keypair, // pays the SOL for the new account
        delegate: Option<Pubkey>,
        account_num: u32,
    ) -> anyhow::Result<(Pubkey, Signature)> {
        let account = Pubkey::find_program_address(
            &[
                b"OpenOrdersAccount".as_ref(),
                owner.pubkey().as_ref(),
                market.as_ref(),
                &account_num.to_le_bytes(),
            ],
            &openbook_v2::id(),
        )
        .0;
        let ix = Instruction {
            program_id: openbook_v2::id(),
            accounts: anchor_lang::ToAccountMetas::to_account_metas(
                &openbook_v2::accounts::InitOpenOrders {
                    owner: owner.pubkey(),
                    open_orders_account: account,
                    payer: payer.pubkey(),
                    delegate_account: delegate,
                    market,
                    system_program: System::id(),
                },
                None,
            ),
            data: anchor_lang::InstructionData::data(&openbook_v2::instruction::InitOpenOrders {
                account_num,
            }),
        };

        let txsig = TransactionBuilder {
            instructions: vec![ix],
            address_lookup_tables: vec![],
            payer: payer.pubkey(),
            signers: vec![owner, payer],
            config: client.transaction_builder_config,
        }
        .send_and_confirm(client)
        .await?;

        Ok((account, txsig))
    }

    /// Conveniently creates a RPC based client
    pub async fn new_for_existing_account(
        client: Client,
        account: Pubkey,
        owner: Arc<Keypair>,
    ) -> anyhow::Result<Self> {
        let rpc = client.rpc_async();
        let account_fetcher = Arc::new(CachedAccountFetcher::new(Arc::new(RpcAccountFetcher {
            rpc,
        })));
        let openbook_account =
            account_fetcher_fetch_openorders_account(&*account_fetcher, &account).await?;
        if openbook_account.owner != owner.pubkey() {
            anyhow::bail!(
                "bad owner for account: expected {} got {}",
                openbook_account.owner,
                owner.pubkey()
            );
        }

        let rpc = client.rpc_async();
        let openbook_context = OpenBookContext::new_from_rpc(&rpc).await?;

        Self::new_detail(client, account, owner, openbook_context, account_fetcher)
    }

    /// Allows control of AccountFetcher and externally created MangoGroupContext
    pub fn new_detail(
        client: Client,
        account: Pubkey,
        owner: Arc<Keypair>,
        // future: maybe pass Arc<MangoGroupContext>, so it can be extenally updated?
        openbook_context: OpenBookContext,
        account_fetcher: Arc<dyn AccountFetcher>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            client,
            account_fetcher,
            owner,
            open_orders_account: account,
            context: openbook_context,
            http_client: reqwest::Client::new(),
        })
    }

    pub fn owner(&self) -> Pubkey {
        self.owner.pubkey()
    }

    pub async fn openorders_account(&self) -> anyhow::Result<OpenOrdersAccount> {
        account_fetcher_fetch_openorders_account(&*self.account_fetcher, &self.open_orders_account)
            .await
    }

    pub async fn get_oracle_price(
        &self,
        oracle: &Pubkey,
    ) -> Result<pyth_sdk_solana::Price, anyhow::Error> {
        let oracle_account = self.account_fetcher.fetch_raw_account(oracle).await?;
        Ok(pyth_sdk_solana::load_price(oracle_account.data()).unwrap())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn place_order(
        &self,
        market_index: MarketIndex,
        side: Side,
        price_lots: i64,
        max_base_lots: i64,
        max_quote_lots_including_fees: i64,
        client_order_id: u64,
        order_type: PlaceOrderType,
        expiry_timestamp: u64,
        limit: u8,
        token_deposit_account: Pubkey,
        market_vault: Pubkey,
        self_trade_behavior: SelfTradeBehavior,
    ) -> anyhow::Result<Signature> {
        let market = self.context.context(market_index);

        let ix = Instruction {
            program_id: openbook_v2::id(),
            accounts: {
                anchor_lang::ToAccountMetas::to_account_metas(
                    &openbook_v2::accounts::PlaceOrder {
                        open_orders_account: self.open_orders_account,
                        open_orders_admin: None,
                        owner_or_delegate: self.owner(),
                        market: market.address,
                        bids: market.market.bids,
                        asks: market.market.asks,
                        event_queue: market.market.event_queue,
                        oracle: market.market.oracle,
                        token_deposit_account,
                        market_vault,
                        system_program: System::id(),
                        token_program: Token::id(),
                    },
                    None,
                )
            },
            data: anchor_lang::InstructionData::data(&openbook_v2::instruction::PlaceOrder {
                side,
                price_lots,
                max_base_lots,
                max_quote_lots_including_fees,
                client_order_id,
                order_type,
                self_trade_behavior,
                expiry_timestamp,
                limit,
            }),
        };
        self.send_and_confirm_owner_tx(vec![ix]).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn place_order_pegged(
        &self,
        market_index: MarketIndex,
        side: Side,
        price_offset_lots: i64,
        peg_limit: i64,
        max_base_lots: i64,
        max_quote_lots_including_fees: i64,
        client_order_id: u64,
        order_type: PlaceOrderType,
        expiry_timestamp: u64,
        limit: u8,
        token_deposit_account: Pubkey,
        market_vault: Pubkey,
        self_trade_behavior: SelfTradeBehavior,
        max_oracle_staleness_slots: i32,
    ) -> anyhow::Result<Signature> {
        let market = self.context.context(market_index);

        let ix = Instruction {
            program_id: openbook_v2::id(),
            accounts: {
                anchor_lang::ToAccountMetas::to_account_metas(
                    &openbook_v2::accounts::PlaceOrder {
                        open_orders_account: self.open_orders_account,
                        open_orders_admin: None,
                        owner_or_delegate: self.owner(),
                        market: market.address,
                        bids: market.market.bids,
                        asks: market.market.asks,
                        event_queue: market.market.event_queue,
                        oracle: market.market.oracle,
                        token_deposit_account,
                        market_vault,
                        system_program: System::id(),
                        token_program: Token::id(),
                    },
                    None,
                )
            },
            data: anchor_lang::InstructionData::data(&openbook_v2::instruction::PlaceOrderPegged {
                side,
                price_offset_lots,
                peg_limit,
                max_oracle_staleness_slots,
                max_base_lots,
                max_quote_lots_including_fees,
                client_order_id,
                order_type,
                self_trade_behavior,
                expiry_timestamp,
                limit,
            }),
        };
        self.send_and_confirm_owner_tx(vec![ix]).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn deposit(
        &self,
        market_index: MarketIndex,
        base_amount: u64,
        quote_amount: u64,
        token_base_account: Pubkey,
        token_quote_account: Pubkey,
        base_vault: Pubkey,
        quote_vault: Pubkey,
    ) -> anyhow::Result<Signature> {
        let market = self.context.context(market_index);

        let ix = Instruction {
            program_id: openbook_v2::id(),
            accounts: {
                anchor_lang::ToAccountMetas::to_account_metas(
                    &openbook_v2::accounts::Deposit {
                        open_orders_account: self.open_orders_account,
                        owner: self.owner(),
                        market: market.address,
                        token_base_account,
                        token_quote_account,
                        base_vault,
                        quote_vault,
                        system_program: System::id(),
                        token_program: Token::id(),
                    },
                    None,
                )
            },
            data: anchor_lang::InstructionData::data(&openbook_v2::instruction::Deposit {
                base_amount,
                quote_amount,
            }),
        };
        self.send_and_confirm_owner_tx(vec![ix]).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn settle_funds(
        &self,
        market_index: MarketIndex,
        token_base_account: Pubkey,
        token_quote_account: Pubkey,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        referrer: Option<Pubkey>,
    ) -> anyhow::Result<Signature> {
        let market = self.context.context(market_index);

        let ix = Instruction {
            program_id: openbook_v2::id(),
            accounts: {
                anchor_lang::ToAccountMetas::to_account_metas(
                    &openbook_v2::accounts::SettleFunds {
                        owner: self.owner(),
                        open_orders_account: self.open_orders_account,
                        market: market.address,
                        token_base_account,
                        token_quote_account,
                        base_vault,
                        quote_vault,
                        referrer,
                        system_program: System::id(),
                        token_program: Token::id(),
                    },
                    None,
                )
            },
            data: anchor_lang::InstructionData::data(&openbook_v2::instruction::SettleFunds {}),
        };
        self.send_and_confirm_owner_tx(vec![ix]).await
    }

    pub async fn send_and_confirm_owner_tx(
        &self,
        instructions: Vec<Instruction>,
    ) -> anyhow::Result<Signature> {
        TransactionBuilder {
            instructions,
            address_lookup_tables: vec![],
            payer: self.client.fee_payer.pubkey(),
            signers: vec![&*self.owner, &*self.client.fee_payer],
            config: self.client.transaction_builder_config,
        }
        .send_and_confirm(&self.client)
        .await
    }

    pub async fn send_and_confirm_permissionless_tx(
        &self,
        instructions: Vec<Instruction>,
    ) -> anyhow::Result<Signature> {
        TransactionBuilder {
            instructions,
            address_lookup_tables: vec![],
            payer: self.client.fee_payer.pubkey(),
            signers: vec![&*self.client.fee_payer],
            config: self.client.transaction_builder_config,
        }
        .send_and_confirm(&self.client)
        .await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenBookClientError {
    #[error("Transaction simulation error. Error: {err:?}, Logs: {}",
        .logs.iter().join("; ")
    )]
    SendTransactionPreflightFailure {
        err: Option<TransactionError>,
        logs: Vec<String>,
    },
}

#[derive(Copy, Clone, Debug)]
pub struct TransactionBuilderConfig {
    // adds a SetComputeUnitPrice instruction in front
    pub prioritization_micro_lamports: Option<u64>,
}

pub struct TransactionBuilder<'a> {
    pub instructions: Vec<Instruction>,
    pub address_lookup_tables: Vec<AddressLookupTableAccount>,
    pub signers: Vec<&'a Keypair>,
    pub payer: Pubkey,
    pub config: TransactionBuilderConfig,
}

impl<'a> TransactionBuilder<'a> {
    pub async fn transaction(
        self,
        rpc: &RpcClientAsync,
    ) -> anyhow::Result<solana_sdk::transaction::VersionedTransaction> {
        let latest_blockhash = rpc.get_latest_blockhash().await?;
        self.transaction_with_blockhash(latest_blockhash)
    }

    pub fn transaction_with_blockhash(
        mut self,
        blockhash: Hash,
    ) -> anyhow::Result<solana_sdk::transaction::VersionedTransaction> {
        if let Some(prio_price) = self.config.prioritization_micro_lamports {
            self.instructions.insert(
                0,
                solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_price(
                    prio_price,
                ),
            )
        }
        let v0_message = solana_sdk::message::v0::Message::try_compile(
            &self.payer,
            &self.instructions,
            &self.address_lookup_tables,
            blockhash,
        )?;
        let versioned_message = solana_sdk::message::VersionedMessage::V0(v0_message);
        let signers = self
            .signers
            .into_iter()
            .unique_by(|s| s.pubkey())
            .collect::<Vec<_>>();
        let tx =
            solana_sdk::transaction::VersionedTransaction::try_new(versioned_message, &signers)?;
        Ok(tx)
    }

    // These two send() functions don't really belong into the transaction builder!

    pub async fn send(self, client: &Client) -> anyhow::Result<Signature> {
        let rpc = client.rpc_async();
        let tx = self.transaction(&rpc).await?;
        rpc.send_transaction_with_config(&tx, client.rpc_send_transaction_config)
            .await
            .map_err(prettify_solana_client_error)
    }

    pub async fn send_and_confirm(self, client: &Client) -> anyhow::Result<Signature> {
        let rpc = client.rpc_async();
        let tx = self.transaction(&rpc).await?;
        // TODO: Wish we could use client.rpc_send_transaction_config here too!
        rpc.send_and_confirm_transaction(&tx)
            .await
            .map_err(prettify_solana_client_error)
    }
}

/// Do some manual unpacking on some ClientErrors
///
/// Unfortunately solana's RpcResponseError will very unhelpfully print [N log messages]
/// instead of showing the actual log messages. This unpacks the error to provide more useful
/// output.
pub fn prettify_client_error(err: anchor_client::ClientError) -> anyhow::Error {
    match err {
        anchor_client::ClientError::SolanaClientError(c) => prettify_solana_client_error(c),
        _ => err.into(),
    }
}

pub fn prettify_solana_client_error(
    err: solana_client::client_error::ClientError,
) -> anyhow::Error {
    use solana_client::client_error::ClientErrorKind;
    use solana_client::rpc_request::{RpcError, RpcResponseErrorData};

    if let ClientErrorKind::RpcError(RpcError::RpcResponseError {
        data: RpcResponseErrorData::SendTransactionPreflightFailure(s),
        ..
    }) = err.kind()
    {
        return OpenBookClientError::SendTransactionPreflightFailure {
            err: s.err.clone(),
            logs: s.logs.clone().unwrap_or_default(),
        }
        .into();
    }

    err.into()
}

#[derive(Clone, Copy)]
pub enum JupiterSwapMode {
    ExactIn,
    ExactOut,
}

pub fn keypair_from_cli(keypair: &str) -> Keypair {
    let maybe_keypair = keypair::read_keypair(&mut keypair.as_bytes());
    match maybe_keypair {
        Ok(keypair) => keypair,
        Err(_) => {
            let path = std::path::PathBuf::from_str(&shellexpand::tilde(keypair)).unwrap();
            keypair::read_keypair_file(path)
                .unwrap_or_else(|_| panic!("Failed to read keypair from {}", keypair))
        }
    }
}

pub fn pubkey_from_cli(pubkey: &str) -> Pubkey {
    match Pubkey::from_str(pubkey) {
        Ok(p) => p,
        Err(_) => keypair_from_cli(pubkey).pubkey(),
    }
}

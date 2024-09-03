use std::fs;
use std::str::FromStr;
use alloy_network::EthereumWallet;
use alloy_provider::ProviderBuilder;
use anyhow::{Result};
use avail_bridge_tools::{AvailBridgeContract, BridgeApiMerkleProof, Config};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use avail_rust::{avail, AvailExtrinsicParamsBuilder, Keypair, SecretUri, WaitFor, H256, SDK};
use avail_rust::avail::vector::calls::types::send_message::Message;

#[tokio::main]
async fn main() -> Result<()> {
    let content = fs::read_to_string("./config.toml").expect("Read config.toml");
    let config = toml::from_str::<Config>(&content).unwrap();

    println!("Using config:\n{:#?}", config);

    let sdk = SDK::new(config.avail_rpc_url.as_str()).await.unwrap();
    let secret_uri = SecretUri::from_str(config.avail_sender_mnemonic.as_str()).unwrap();
    let account = Keypair::from_uri(&secret_uri).unwrap();

    // Ethereum domain
    let domain = 2u32;
    // Recipient address on the Ethereum network
    let recipient = config.recipient.parse()?;

    // Fungible token message to send
    let message = Message::FungibleToken {
        asset_id: H256::zero(),
        amount: config.amount_to_send as u128,
    };

    let da_call = avail::tx().vector().send_message(
        message,
        recipient,
        domain,
    );
    let params = AvailExtrinsicParamsBuilder::new().build();
    let maybe_tx_progress = sdk
        .api
        .tx()
        .sign_and_submit_then_watch(&da_call, &account, params)
        .await;

    let transaction = sdk
        .util
        .progress_transaction(maybe_tx_progress, WaitFor::BlockFinalization)
        .await;

    let tx_in_block = match transaction {
        Ok(tx_in_block) => tx_in_block,
        Err(message) => {
            panic!("Error: {}", message);
        }
    };

    println!("Finalized block hash: {:?}", tx_in_block.block_hash());
    let events = tx_in_block.wait_for_success().await.unwrap();
    println!("Transaction result: {:?}", events);

    let block_hash = tx_in_block.block_hash();
    let extrinsic_index = events.extrinsic_index();

    let block = sdk
        .rpc
        .chain
        .get_block(None)
        .await.unwrap();

    let block_num = block.block.header.number;

    loop {
        let avail_head_info: AvailHeadInfo = reqwest::get(format!("{}/avl/head", config.bridge_api_url))
            .await
            .unwrap()
            .json()
            .await?;
        println!("New range: {avail_head_info:?}");

        if (avail_head_info.data.start..=avail_head_info.data.end).contains(&(block_num as u64)) {
            println!("Stored avail head is in range!");
            break;
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }

    let url: String = format!(
        "{}/eth/proof/{:?}?index={}",
        config.bridge_api_url, block_hash, extrinsic_index
    );
    println!("Proof url: {url}");
    let proof: BridgeApiMerkleProof = reqwest::get(url).await.unwrap().json().await.unwrap();

    println!("Proof: {proof:?}");
    let signer = config.ethereum_secret.parse::<alloy_signer_local::PrivateKeySigner>()?;
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(EthereumWallet::from(signer))
        .on_http(Url::parse(config.ethereum_url.as_str())?);

    let contract_address = config.contract_address.parse()?;

    let contract = AvailBridgeContract::new(contract_address, &provider);

    let call = contract.receiveAVAIL(proof.clone().try_into().unwrap(), proof.into());
    let pending_tx = call.send().await?;
    let res = pending_tx.watch().await?;
    println!("Result: {res:?}");

    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct AvailHeadInfo {
    data: AvailHeadData,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct AvailHeadData {
    start: u64,
    end: u64,
}

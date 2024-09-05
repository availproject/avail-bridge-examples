use alloy_network::EthereumWallet;
use alloy_provider::ProviderBuilder;
use anyhow::Result;
use avail_bridge_tools::{address_to_h256, AvailBridgeContract, BridgeApiMerkleProof, Config};
use avail_rust::avail::runtime_types::bounded_collections::bounded_vec::BoundedVec;
use avail_rust::avail::vector::calls::types::send_message::Message;
use avail_rust::{avail, AvailExtrinsicParamsBuilder, Keypair, SecretUri, WaitFor, SDK};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::fs;
use std::str::FromStr;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let content = fs::read_to_string("./config.toml").expect("Read config.toml");
    let config = toml::from_str::<Config>(&content).expect("Parse config.toml");

    println!("Using config:\n{:#?}", config);

    let sdk = SDK::new(config.avail_rpc_url.as_str())
        .await
        .expect("Initializing SDK");
    let secret_uri =
        SecretUri::from_str(config.avail_sender_mnemonic.as_str()).expect("Valid secret URI");
    let account = Keypair::from_uri(&secret_uri).expect("Valid secret URI");

    // Ethereum domain
    let domain = 2u32;
    // Recipient contract address on the Ethereum network
    let recipient = address_to_h256(config.receive_message_contract_address.parse()?);

    let data = BoundedVec(config.message_data.as_bytes().to_vec());
    // Arbitrary message to send
    let message = Message::ArbitraryMessage(data);

    let da_call = avail::tx()
        .vector()
        .send_message(message, recipient, domain);
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
    let events = tx_in_block
        .wait_for_success()
        .await
        .expect("Waiting for success");
    println!("Transaction result: {:?}", events);

    let block_hash = tx_in_block.block_hash();
    let extrinsic_index = events.extrinsic_index();

    let block = sdk
        .rpc
        .chain
        .get_block(None)
        .await
        .expect("Get block by hash");

    let block_num = block.block.header.number;
    loop {
        let avail_head_info: AvailHeadInfo =
            reqwest::get(format!("{}/avl/head", config.bridge_api_url))
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
    let signer = config
        .ethereum_secret
        .parse::<alloy_signer_local::PrivateKeySigner>()?;
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(EthereumWallet::from(signer))
        .on_http(Url::parse(config.ethereum_url.as_str())?);

    let contract_address = config.contract_address.parse()?;

    let contract = AvailBridgeContract::new(contract_address, &provider);

    let call = contract.receiveMessage(proof.clone().try_into().unwrap(), proof.into());
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

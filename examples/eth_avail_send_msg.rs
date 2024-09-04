use std::fs;
use std::str::FromStr;
use alloy::primitives::Address;
use alloy_network::EthereumWallet;
use alloy_provider::ProviderBuilder;
use alloy_sol_types::sol;
use anyhow::{anyhow, Result};
use avail_bridge_tools::{address_to_h256, convert_addressed_message, eth_seed_to_address, Config};
use reqwest::Url;
use serde::{Deserialize, Deserializer};
use sp_core::H256;
use std::time::Duration;
use avail_rust::{avail, AvailConfig, AvailExtrinsicParamsBuilder, Keypair, SecretUri, WaitFor, SDK};
use avail_rust::avail::runtime_types::bounded_collections::bounded_vec::BoundedVec;
use avail_rust::avail_core::data_proof::BoundedData;
use avail_rust::sp_core::Pair as PairT;
use avail_rust::sp_core::sr25519::Pair;

sol!(
    #[sol(rpc)]
    AvailBridgeContract,
    "src/availbridge.json"
);

#[tokio::main]
async fn main() -> Result<()> {
    let content = fs::read_to_string("./config.toml").expect("Read config.toml");
    let config = toml::from_str::<Config>(&content).unwrap();

    let sender = PairT::from_string_with_seed(config.avail_sender_mnemonic.as_str(), None).unwrap();
    let avail_signer = PairSigner::<AvailConfig, Pair>::new(sender.clone().0);

    let secret_uri = SecretUri::from_str(config.avail_sender_mnemonic.as_str()).unwrap();
    let account = Keypair::from_uri(&secret_uri).unwrap();


    let recipient = sender.0.public().0;

    let ethereum_signer = config.ethereum_secret.parse::<alloy_signer_local::PrivateKeySigner>()?;

    let sender = eth_seed_to_address(config.ethereum_secret.as_str());
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(EthereumWallet::from(ethereum_signer))
        .on_http(Url::parse(config.ethereum_url.as_str())?);

    let contract_addr: Address = config.contract_address.parse()?;

    let contract = AvailBridgeContract::new(contract_addr, &provider);

    let call = contract.sendMessage(recipient.into(), config.message_data.into());
    let pending_tx = call.from(sender.0.into());
    let pending_tx = pending_tx.send().await?;
    let receipt = pending_tx.get_receipt().await?;
    let block_number = receipt.block_number.ok_or(anyhow!("No block number!"))?;
    println!("Included in block no: {block_number}");
    let logs = receipt
        .inner
        .as_receipt()
        .ok_or(anyhow!("Cannot convert to receipt"))?
        .logs
        .clone();
    assert!(!logs.is_empty(), "Logs are empty!");

    let message_id = u64::from_be_bytes(
        logs[0].clone().inner.data.data[32 - 8..]
            .try_into()
            .unwrap(),
    );

    let sent_message = avail_rust::avail_core::data_proof::AddressedMessage {
        message: avail_rust::avail_core::data_proof::message::Message::ArbitraryMessage(BoundedData::truncate_from(config.message_data.as_bytes().to_vec())),
        from: address_to_h256(sender),
        to: H256(recipient),
        origin_domain: 2,
        destination_domain: 1,
        id: message_id,
    };

    let (avail_stored_block_hash, avail_stored_slot) = loop {
        let ethereum_slot_info: EthereumSlotInfo =
            reqwest::get(format!("{}/eth/head", config.bridge_api_url))
                .await
                .unwrap()
                .json()
                .await?;
        println!("New slot: {ethereum_slot_info:?}");
        let block_info: BlockInfo = reqwest::get(format!(
            "{}/beacon/slot/{}",
            config.bridge_api_url, ethereum_slot_info.slot
        ))
            .await
            .unwrap()
            .json()
            .await?;
        println!("Slot to num: {}", block_info.block_number);
        if block_info.block_number >= block_number {
            println!("Stored eth head is in range!");
            break (block_info.block_hash, ethereum_slot_info.slot);
        }

        tokio::time::sleep(Duration::from_secs(60)).await;
    };

    let account_storage_proof: AccountStorageProof = reqwest::get(format!(
        "{}/avl/proof/{:?}/{}",
        config.bridge_api_url, avail_stored_block_hash, message_id
    ))
        .await
        .expect("Cannot get account/storage proofs.")
        .json()
        .await
        .expect("Cannot deserialize");
    println!("Got proof! {account_storage_proof:?}");

    let acc_proof = BoundedVec(
        account_storage_proof
            .account_proof
            .into_iter()
            .map(BoundedVec)
            .collect::<Vec<_>>(),
    );
    let stor_proof = BoundedVec(
        account_storage_proof
            .storage_proof
            .into_iter()
            .map(BoundedVec)
            .collect::<Vec<_>>(),
    );

    println!("Message: {sent_message:?}");
    let sdk = SDK::new(config.avail_rpc_url.as_str()).await.unwrap();

    let da_call = avail::tx().vector().execute(
        avail_stored_slot,
        convert_addressed_message(sent_message),
        acc_proof,
        stor_proof,
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


    // let tx = avail_subxt::api::tx().vector().execute(
    //     avail_stored_slot,
    //     convert_addressed_message(sent_message),
    //     acc_proof,
    //     stor_proof,
    // );
    //
    // let client = avail_subxt::AvailClient::new(avail_rpc_url).await.unwrap();
    //
    // let executed_block_hash = client
    //     .tx()
    //     .sign_and_submit_then_watch_default(&tx, &avail_signer)
    //     .await?
    //     .wait_for_finalized_success()
    //     .await?
    //     .block_hash();
    //
    // println!("Executed at block: {executed_block_hash:?}");

    Ok(())
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct BlockInfo {
    block_number: u64,
    block_hash: H256,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct EthereumSlotInfo {
    pub slot: u64,
    pub _timestamp: u64,
    pub _timestamp_diff: u64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct AccountStorageProof {
    #[serde(deserialize_with = "bytes_from_hex")]
    account_proof: Vec<Vec<u8>>,
    #[serde(deserialize_with = "bytes_from_hex")]
    storage_proof: Vec<Vec<u8>>,
}

fn bytes_from_hex<'de, D>(deserializer: D) -> Result<Vec<Vec<u8>>, D::Error>
where
    D: Deserializer<'de>,
{
    let buf = <Vec<String>>::deserialize(deserializer)?;
    let res = buf
        .iter()
        .map(|e| {
            let without_prefix = e.trim_start_matches("0x");
            hex::decode(without_prefix).unwrap()
        })
        .collect::<Vec<_>>();

    Ok(res)
}

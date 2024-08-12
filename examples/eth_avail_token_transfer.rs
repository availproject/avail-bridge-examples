use alloy::primitives::{Address, U256};
use alloy_network::EthereumWallet;
use alloy_provider::ProviderBuilder;
use alloy_sol_types::sol;
use anyhow::{anyhow, Result};
use avail_bridge_tools::{address_to_h256, convert_addressed_message, eth_seed_to_address};
use avail_core::data_proof::AddressedMessage;
use avail_subxt::{AvailConfig, BoundedVec};
use reqwest::Url;
use serde::{Deserialize, Deserializer};
use sp_core::H256;
use std::time::Duration;
use subxt::ext::sp_core::sr25519::Pair;
use subxt::ext::sp_core::Pair as PairT;
use subxt::tx::PairSigner;

sol!(
    #[sol(rpc)]
    AvailBridgeContract,
    "src/availbridge.json"
);

#[tokio::main]
async fn main() -> Result<()> {
    let avail_rpc_url = "wss://rpc-hex-devnet.avail.tools:443/ws";
    let avail_sender_mnemonic =
        "bottom drive obey lake curtain smoke basket hold race lonely fit walk//Alice";
    let ethereum_secret = "YOUR_SECRET_SEED";
    let bridge_api_url = "https://hex-bridge-api.sandbox.avail.tools";
    let ethereum_url = "https://ethereum-sepolia.publicnode.com";
    let contract_address = "1369A4C9391cF90D393b40fAeAD521b0F7019dc5";
    let sender = PairT::from_string_with_seed(avail_sender_mnemonic, None).unwrap();
    let avail_signer = PairSigner::<AvailConfig, Pair>::new(sender.clone().0);

    let recipient = sender.0.public().0;
    let amount: u128 = 100000;

    let ethereum_signer = ethereum_secret.parse::<alloy_signer_local::PrivateKeySigner>()?;

    let sender = eth_seed_to_address(ethereum_secret);
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(EthereumWallet::from(ethereum_signer))
        .on_http(Url::parse(ethereum_url)?);

    let contract_addr: Address = contract_address.parse()?;

    let contract = AvailBridgeContract::new(contract_addr, &provider);

    let call = contract.sendAVAIL(recipient.into(), U256::from(amount));
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

    let sent_message = AddressedMessage {
        message: avail_core::data_proof::Message::FungibleToken {
            asset_id: H256::zero(),
            amount,
        },
        from: address_to_h256(sender),
        to: H256(recipient),
        origin_domain: 2,
        destination_domain: 1,
        id: message_id,
    };

    let (avail_stored_block_hash, avail_stored_slot) = loop {
        let ethereum_slot_info: EthereumSlotInfo =
            reqwest::get(format!("{}/eth/head", bridge_api_url))
                .await
                .unwrap()
                .json()
                .await?;
        println!("New slot: {ethereum_slot_info:?}");
        let block_info: BlockInfo = reqwest::get(format!(
            "{}/beacon/slot/{}",
            bridge_api_url, ethereum_slot_info.slot
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
        bridge_api_url, avail_stored_block_hash, message_id
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

    let tx = avail_subxt::api::tx().vector().execute(
        avail_stored_slot,
        convert_addressed_message(sent_message),
        acc_proof,
        stor_proof,
    );

    let client = avail_subxt::AvailClient::new(avail_rpc_url).await.unwrap();

    let executed_block_hash = client
        .tx()
        .sign_and_submit_then_watch_default(&tx, &avail_signer)
        .await?
        .wait_for_finalized_success()
        .await?
        .block_hash();

    println!("Executed at block: {executed_block_hash:?}");

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

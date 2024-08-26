use alloy_network::EthereumWallet;
use alloy_provider::ProviderBuilder;
use anyhow::{Context, Result};
use avail_bridge_tools::{AvailBridgeContract, BridgeApiMerkleProof};
use avail_core::AppId;
use avail_subxt::{AvailConfig, BoundedVec};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use subxt::ext::sp_core::sr25519::Pair;
use subxt::ext::sp_core::Pair as PairT;
use subxt::tx::PairSigner;

#[tokio::main]
async fn main() -> Result<()> {
    let avail_rpc_url = "wss://rpc-hex-devnet.avail.tools:443/ws";
    let avail_sender_mnemonic =
        "bottom drive obey lake curtain smoke basket hold race lonely fit walk//Alice";
    let ethereum_secret = "YOUR_SECRET_SEED";
    let bridge_api_url = "https://hex-bridge-api.sandbox.avail.tools";
    let ethereum_url = "https://ethereum-sepolia.publicnode.com";
    let contract_address = "1369A4C9391cF90D393b40fAeAD521b0F7019dc5";
    let message_data = b"Example data";

    let client = avail_subxt::AvailClient::new(avail_rpc_url).await.unwrap();

    let sender = PairT::from_string_with_seed(avail_sender_mnemonic, None).unwrap();
    let signer = PairSigner::<AvailConfig, Pair>::new(sender.0);

    let tx = avail_subxt::api::tx()
        .data_availability()
        .submit_data(BoundedVec(message_data.to_vec()));

    let e_event = client
        .tx()
        .sign_and_submit_then_watch(
            &tx,
            &signer,
            avail_subxt::primitives::new_params_from_app_id(AppId(1)),
        )
        .await
        .context("Submission failed")
        .unwrap()
        .wait_for_finalized_success()
        .await
        .context("Waiting for success failed")
        .unwrap();
    let block_hash = e_event.block_hash();
    let extrinsic_index = e_event.extrinsic_index();

    let block_num = client.blocks().at(block_hash).await.unwrap().number();
    println!("DA transaction included in block: {block_num}, hash: {block_hash:?}, index:{extrinsic_index}");

    loop {
        let avail_head_info: AvailHeadInfo = reqwest::get(format!("{}/v1/avl/head", bridge_api_url))
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
        "{}/v1/eth/proof/{:?}?index={}",
        bridge_api_url, block_hash, extrinsic_index
    );
    println!("Proof url: {url}");
    let proof: BridgeApiMerkleProof = reqwest::get(url).await.unwrap().json().await.unwrap();

    println!("Proof: {proof:?}");
    let signer = ethereum_secret.parse::<alloy_signer_local::PrivateKeySigner>()?;
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(EthereumWallet::from(signer))
        .on_http(Url::parse(ethereum_url)?);

    let contract_address = contract_address.parse()?;

    let contract = AvailBridgeContract::new(contract_address, &provider);

    let call = contract.verifyBlobLeaf(proof.into());
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

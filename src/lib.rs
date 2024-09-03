use alloy::primitives::Address;
use alloy::primitives::Bytes;
use alloy::primitives::FixedBytes;
use alloy::primitives::Uint;
use alloy_sol_types::sol;
use avail_rust::avail_core::data_proof::{AddressedMessage, BoundedData};
use serde::Deserialize;
use sp_core::H256;
use avail_rust::avail_core::data_proof::message::Message as AvailBridgeMessage;

pub const ABI_JSON: &[u8] = include_bytes!("availbridge.json");

sol!(
    #[sol(rpc)]
    AvailBridgeContract,
    "src/availbridge.json"
);

pub fn convert_addressed_message(message: AddressedMessage) -> AddressedMessage {
    let msg = match message.message {
        avail_rust::avail_core::data_proof::Message::ArbitraryMessage(data) => {
            AvailBridgeMessage::ArbitraryMessage(BoundedData::truncate_from(data.to_vec()))
        }
        avail_rust::avail_core::data_proof::Message::FungibleToken { asset_id, amount } => {
            AvailBridgeMessage::FungibleToken { asset_id, amount }
        }
    };
    AddressedMessage {
        message: msg,
        from: message.from,
        to: message.to,
        origin_domain: message.origin_domain,
        destination_domain: message.destination_domain,
        id: message.id,
    }
}

pub fn enc_value_to_amount(data: &[u8]) -> u128 {
    u128::from_be_bytes(data[64 - 16..].try_into().unwrap())
}

pub fn enc_amount_to_value(amount: u128) -> Vec<u8> {
    let mut data = vec![0; 64 - 16];
    data.extend(amount.to_be_bytes());
    data
}

pub fn eth_seed_to_address(seed: &str) -> Address {
    let ethereum_signer = seed
        .parse::<alloy_signer_local::PrivateKeySigner>()
        .unwrap();
    ethereum_signer.address()
}

pub fn address_to_h256(from: Address) -> H256 {
    let mut v = from.0.to_vec();
    v.resize(32, 0);
    H256(v.as_slice().try_into().unwrap())
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BridgeApiMerkleProof {
    pub blob_root: H256,
    pub block_hash: H256,
    pub bridge_root: H256,
    pub data_root: H256,
    pub data_root_commitment: H256,
    pub data_root_index: u32,
    pub data_root_proof: Vec<H256>,
    pub leaf: H256,
    pub leaf_index: u32,
    pub leaf_proof: Vec<H256>,
    pub message: Option<AddressedMessage>,
    pub range_hash: H256,
}
impl TryFrom<BridgeApiMerkleProof> for AvailBridgeContract::Message {
    type Error = &'static str;
    fn try_from(value: BridgeApiMerkleProof) -> Result<Self, Self::Error> {
        let Some(message) = value.message else {
            return Err("Message not found");
        };
        let (msg_type, data) = match message.message {
            avail_rust::avail_core::data_proof::Message::ArbitraryMessage(data) => (1u8, data.to_vec()),
            avail_rust::avail_core::data_proof::Message::FungibleToken {
                asset_id: _,
                amount,
            } => (2u8, enc_amount_to_value(amount)),
        };
        Ok(Self {
            messageType: FixedBytes::from_slice(&[msg_type]),
            from: message.from.0.into(),
            to: message.to.0.into(),
            originDomain: message.origin_domain,
            destinationDomain: message.destination_domain,
            data: Bytes::copy_from_slice(data.as_slice()),
            messageId: message.id,
        })
    }
}

impl From<BridgeApiMerkleProof> for AvailBridgeContract::MerkleProofInput {
    fn from(value: BridgeApiMerkleProof) -> Self {
        AvailBridgeContract::MerkleProofInput {
            dataRootProof: value
                .data_root_proof
                .into_iter()
                .map(|e| FixedBytes::from_slice(e.0.as_slice()))
                .collect(),
            leafProof: value
                .leaf_proof
                .into_iter()
                .map(|e| FixedBytes::from_slice(e.0.as_slice()))
                .collect(),
            rangeHash: value.range_hash.0.into(),
            dataRootIndex: Uint::from(value.data_root_index),
            blobRoot: FixedBytes::from_slice(value.blob_root.0.as_slice()),
            bridgeRoot: value.bridge_root.0.into(),
            leaf: value.leaf.0.into(),
            leafIndex: Uint::from(value.leaf_index),
        }
    }
}


#[derive(Debug, Deserialize)]
pub struct Config {
    pub avail_rpc_url: String,
    pub avail_sender_mnemonic: String,
    pub ethereum_secret: String,
    pub bridge_api_url: String,
    pub ethereum_url: String,
    pub contract_address: String,
    pub message_data: String,
    pub amount_to_send: u64,
    pub recipient: String,
    pub receive_message_contract_address: String,
}

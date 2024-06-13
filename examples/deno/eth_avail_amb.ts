import {ApiPromise, Keyring, WsProvider} from "https://deno.land/x/polkadot@0.2.45/api/mod.ts";
import {API_EXTENSIONS, API_RPC, API_TYPES} from "./api_options.ts";
import {ISubmittableResult} from "https://deno.land/x/polkadot@0.2.45/types/types/extrinsic.ts";
import {ethers} from "npm:ethers@5.4";
import ABI from './abi/availbridge.json' with {type: "json"};
import {BN} from "https://deno.land/x/polkadot@0.2.45/util/mod.ts";
import {encodeAbiParameters} from 'npm:viem'

const AVAIL_RPC = "wss://turing-rpc.avail.so/ws";
const SURI = "YOUR_SECRET_SEED";
const BRIDGE_ADDRESS = "0x967F7DdC4ec508462231849AE81eeaa68Ad01389"; // deployed bridge address
const BRIDGE_API_URL = "https://turing-bridge-api.fra.avail.so"; // bridge api url
const ETH_PROVIDER_URL = "https://ethereum-sepolia.publicnode.com"; // eth provider url
const WALLET_SIGNER_KEY = "ETHEREUM_WALLET_SIGNER_KEY";
const FROM = "ETHEREUM_FROM_ADDRESS_FROM_SIGNER"; // address as 32 bytes
const TO = "AVAIL_WALLET_KEY";
const DATA_TO_SEND = "0x1234";

const AVAIL_API = await ApiPromise.create({
    provider: new WsProvider(AVAIL_RPC),
    rpc: API_RPC,
    types: API_TYPES,
    signedExtensions: API_EXTENSIONS,
});
const ACCOUNT = new Keyring({type: "sr25519"}).addFromUri(SURI);

/**
 *  ProofData represents account and storage proofs from the Ethereum network.
 */
class ProofData {
    accountProof: Array<string>
    storageProof: Array<string>
}

/**
 * sendMessage invokes sendMessage bridge contract functions.
 *
 */
async function sendMessage() {
    console.log("Sending transaction...")
    const provider = new ethers.providers.JsonRpcProvider(ETH_PROVIDER_URL);
    const signer = new ethers.Wallet(WALLET_SIGNER_KEY, provider);
    const contractInstance = new ethers.Contract(BRIDGE_ADDRESS, ABI, signer);

    const response = await contractInstance.sendMessage(
        TO,
        DATA_TO_SEND
    );

    return await response.wait();
}


let receipt = await sendMessage()

// get message id from the receipt event after successful transaction execution.
let messageId = receipt.events[0].args[2].toNumber();

console.log(`Transaction sent ${receipt.blockNumber} and message id ${messageId} `)

while (true) {
    let getHeadRsp = await fetch(BRIDGE_API_URL + "/eth/head");
    if (getHeadRsp.status != 200) {
        console.log("Something went wrong fetching the head.");
        break;
    }
    let headRsp = await getHeadRsp.json();
    let txSendBlockNumber: number = receipt.blockNumber;
    let slot: number = headRsp.slot;
    // map slot number to block number
    let slotMappingRsp = await fetch(BRIDGE_API_URL + "/beacon/slot/" + slot);
    let mappingResponse = await slotMappingRsp.json();
    console.log(`Block inclusion number ${txSendBlockNumber}, head block number ${mappingResponse.blockNumber}`);
    // check if we can execute message
    // if the head on a pallet is updated with a block number >= block number when tx was sent
    if (mappingResponse.blockNumber >= txSendBlockNumber) {
        console.log("Fetching the blob proof.")
        const proofResponse = await fetch(BRIDGE_API_URL + "/avl/proof/" + mappingResponse.blockHash + "/" + messageId);
        if (proofResponse.status != 200) {
            console.log("Something went wrong fetching the proof.")
            console.log(proofResponse)
            break;
        }
        let proof: ProofData = await proofResponse.json();
        // call the deployed contract function with the inclusion proof and the message that was sent.
        const rsp = await new Promise<ISubmittableResult>((res) => {
            AVAIL_API.tx.vector.execute(
                slot,
                {
                    message: {
                        ArbitraryMessage: DATA_TO_SEND
                    },
                    from: FROM,
                    to: TO,
                    originDomain: 2, // eth domain
                    destinationDomain: 1, // avail domain
                    id: messageId,
                },
                proof.accountProof,
                proof.storageProof
            ).signAndSend(ACCOUNT, {nonce: -1}, (result: ISubmittableResult) => {
                console.log(`Tx status: ${result.status}`)
                if (result.isError) {
                    console.log(`Tx failed!`);
                    res(result)
                }
                if (result.isInBlock) {
                    console.log("Transaction in block, waiting for block finalization...")
                }
                if (result.isFinalized) {
                    console.log(`Tx finalized.`)
                    res(result)
                }
            });
        });
        console.log(`Transaction included in block number: ${rsp.blockNumber}`)
        break;
    }

    console.log(`Waiting to bridge inclusion commitment. This can take a while...`)
    // wait for 1 minute to check again
    await new Promise(f => setTimeout(f, 60 * 1000));
}

Deno.exit(0);

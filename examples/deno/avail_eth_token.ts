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
const TOKEN_TO_SEND = new BN("1000000000000000000");
const TO = "ETHEREUM_TO_ADDRESS"; // eth address as 32 bytes

const AVAIL_API = await ApiPromise.create({
    provider: new WsProvider(AVAIL_RPC),
    rpc: API_RPC,
    types: API_TYPES,
    signedExtensions: API_EXTENSIONS,
});
const ACCOUNT = new Keyring({type: "sr25519"}).addFromUri(SURI);

/**
 *  ProofData represents a response from the api that holds proof for
 *  the verification with a token message that was sent.
 */
class ProofData {
    dataRootProof: Array<string>
    leafProof: string
    rangeHash: string
    dataRootIndex: number
    blobRoot: string
    bridgeRoot: string
    leaf: string
    leafIndex: number
    message: Message
}

/**
 * Message represent token sent information.
 */
class Message {
    destinationDomain: number
    from: string
    id: number
    message: any
    originDomain: number
    to: string
}

/**
 * Submitting message extrinsic call.
 */
async function sendToken() {
    return await new Promise<ISubmittableResult>((res) => {
        console.log("Sending transaction...")
        AVAIL_API.tx.vector.sendMessage({
                FungibleToken: {
                    assetId: "0x0000000000000000000000000000000000000000000000000000000000000000",
                    amount: TOKEN_TO_SEND
                }
            },
            TO, // address to send tokens
            2 // eth domain
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
        })
    });
}


let result = await sendToken();
if (result.isFinalized) {
    console.log(`Message transaction in finalized block: ${result.blockNumber}, transaction index: ${result.txIndex}`);
} else {
    console.log("Something went wrong!");
    console.log(result);
    Deno.exit(0);
}

// wait until the chain head on the Ethereum network is updated with the block range
// in which the Avail token bridge transaction is included.
while (true) {
    let getHeadRsp = await fetch(BRIDGE_API_URL + "/v1/avl/head");
    if (getHeadRsp.status != 200) {
        console.log("Something went wrong fetching the head.");
        break;
    }
    let headRsp = await getHeadRsp.json();
    let txBlockNumber: number = result.blockNumber.toNumber();
    let lastCommittedBlock: number = headRsp.data.end;
    // wait until last committed block is >= from the block number where transaction was included
    if (lastCommittedBlock >= txBlockNumber) {
        console.log("Fetching the proof...")
        const proofResponse =
            await fetch(BRIDGE_API_URL + "/v1/eth/proof/" + result.status.asFinalized + "?index=" + result.txIndex);
        if (proofResponse.status != 200) {
            console.log("Something went wrong fetching the proof.")
            console.log(proofResponse)
            break;
        }
        let proof: ProofData = await proofResponse.json();
        // call the deployed contract function with the inclusion proof.
        const provider = new ethers.providers.JsonRpcProvider(ETH_PROVIDER_URL);
        const signer = new ethers.Wallet(WALLET_SIGNER_KEY, provider);

        const contractInstance = new ethers.Contract(BRIDGE_ADDRESS, ABI, signer);
        const receipt = await contractInstance.receiveAVAIL(
            [
                "0x02", // token transfer type
                proof.message.from,
                proof.message.to,
                proof.message.originDomain,
                proof.message.destinationDomain,
                encodeAbiParameters(
                    [
                        {
                            name: "assetId",
                            type: "bytes32",
                        },
                        {
                            name: "amount",
                            type: "uint256",
                        },
                    ],
                    [
                        proof.message.message.fungibleToken.asset_id,
                        proof.message.message.fungibleToken.amount,
                    ])
                ,
                proof.message.id
            ], [
                proof.dataRootProof,
                proof.leafProof,
                proof.rangeHash,
                proof.dataRootIndex,
                proof.blobRoot,
                proof.bridgeRoot,
                proof.leaf,
                proof.leafIndex]
        );
        const received = await receipt.wait();
        console.log(received)
        console.log(`Receive avail in block number: ${received.blockNumber}`)
        break;
    }

    console.log(`Waiting to bridge inclusion commitment, last block on head ${lastCommittedBlock}. This can take a while...`)
    // wait for 1 minute to check again
    await new Promise(f => setTimeout(f, 60 * 1000));
}

Deno.exit(0);



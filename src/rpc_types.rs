use serde::{Deserialize, Serialize};

use sp_core::{Encode, H256};
use sp_runtime::Justifications;

use crate::{Block, Runtime};

pub type Properties = serde_json::map::Map<String, serde_json::Value>;

pub type Header = <Runtime as frame_system::Config>::Header;
pub type Index = <Runtime as frame_system::Config>::Index;
pub type AccountData = <Runtime as frame_system::Config>::AccountData;
pub type BlockNumber = <Runtime as frame_system::Config>::BlockNumber;

pub type Number = u64;
pub type Hash = H256;
pub type BlockHash = H256;

pub type Bytes = String;

pub type StorageKey = String;
pub type StorageData = String;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct MockBlock {
    pub header: Header,
    pub extrinsics: Vec<Bytes>,
}

impl MockBlock {
    fn new(block: Block) -> Self {
        Self {
            header: block.header,
            extrinsics: block
                .extrinsics
                .iter()
                .map(|xt| format!("0x{}", hex::encode(&xt.encode()[..])))
                .collect(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct MockSignedBlock {
    /// Full block.
    pub block: MockBlock,
    /// Block justification.
    pub justifications: Option<Justifications>,
}

impl MockSignedBlock {
    pub fn new(block: Block, justifications: Option<Justifications>) -> Self {
        Self {
            block: MockBlock::new(block),
            justifications,
        }
    }
}

pub type SignedBlock = MockSignedBlock;

/// Storage change set
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct StorageChangeSet<Hash> {
    /// Block hash
    pub block: Hash,
    /// A list of changes
    pub changes: Vec<(StorageKey, Option<StorageData>)>,
}

#[derive(Copy, Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(untagged)]
pub enum NumberOrHex {
    /// The number represented directly.
    Number(BlockNumber),
    /// Hex representation of the number.
    Hex(BlockHash),
}

impl Default for NumberOrHex {
    fn default() -> Self {
        Self::Number(Default::default())
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct RuntimeVersion {
    pub spec_name: String,
    pub impl_name: String,
    pub authoring_version: u64,
    pub spec_version: u64,
    pub impl_version: u64,
    pub apis: Vec<([u8; 8], u32)>,
    pub transaction_version: u64,
    pub state_version: u8,
}

/// Available RPC methods.
#[derive(Copy, Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(untagged)]
pub enum RpcMethods {
    Auto,
    Safe,
    Unsafe,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub enum ChainType {
    /// A development chain that runs mainly on one node.
    Development,
    /// A local chain that runs locally on multiple nodes for testing purposes.
    Local,
    /// A live chain.
    Live,
    /// Some custom chain type.
    Custom(String),
}

/// Wrapper functions to keep the API backwards compatible over the wire for the old RPC spec.
mod v1_compatible {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S, H>(data: &(H, usize), serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        H: Serialize,
    {
        let (hash, _) = data;
        serde::Serialize::serialize(&hash, serializer)
    }

    pub fn deserialize<'de, D, H>(deserializer: D) -> Result<(H, usize), D::Error>
    where
        D: Deserializer<'de>,
        H: Deserialize<'de>,
    {
        let hash: H = serde::Deserialize::deserialize(deserializer)?;
        Ok((hash, 0))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransactionStatus<Hash, BlockHash> {
    /// Transaction is part of the future queue.
    Future,
    /// Transaction is part of the ready queue.
    Ready,
    /// The transaction has been broadcast to the given peers.
    Broadcast(Vec<String>),
    /// Transaction has been included in block with given hash
    /// at the given position.
    #[serde(with = "v1_compatible")]
    InBlock((BlockHash, usize)),
    /// The block this transaction was included in has been retracted.
    Retracted(BlockHash),
    /// Maximum number of finality watchers has been reached,
    /// old watchers are being removed.
    FinalityTimeout(BlockHash),
    /// Transaction has been finalized by a finality-gadget, e.g GRANDPA.
    #[serde(with = "v1_compatible")]
    Finalized((BlockHash, usize)),
    /// Transaction has been replaced in the pool, by another transaction
    /// that provides the same tags. (e.g. same (sender, nonce)).
    Usurped(Hash),
    /// Transaction has been dropped from the pool because of the limit.
    Dropped,
    /// Transaction is no longer valid in the current state.
    Invalid,
}

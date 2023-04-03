use jsonrpsee::{
    core::{async_trait, client::ClientT, server::rpc_module::SubscriptionSink, RpcResult},
    proc_macros::rpc,
    rpc_params,
    types::SubscriptionResult,
    ws_client::WsClientBuilder,
};

use futures::{future, stream, FutureExt, StreamExt};
use std::{
    sync::{Arc, Mutex},
    thread, time,
};

use sc_client_api::StorageNotifications;
use sp_state_machine::InMemoryBackend;

use sp_core::{blake2_256, Blake2Hasher, Encode, H256};
use sp_runtime::{traits::Block as BlockT, Justifications};

use crate::{account::AccountId, Block, BlockNumber, Database, Runtime, TargetBlock, ENDPOINT};
use serde::{Deserialize, Serialize};
use sp_state_machine::backend::Backend;

pub type Properties = serde_json::map::Map<String, serde_json::Value>;

pub type Header = <Runtime as frame_system::Config>::Header;
pub type Index = <Runtime as frame_system::Config>::Index;

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
    fn new(block: TargetBlock) -> Self {
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
    fn new(block: TargetBlock, justifications: Option<Justifications>) -> Self {
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
    pub authoring_version: u32,
    pub spec_version: u32,
    pub impl_version: u32,
    pub apis: Vec<([u8; 8], u32)>,
    pub transaction_version: u32,
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

#[rpc(server)]
pub trait MockApi<AccountId, Number, Hash, Header, BlockHash, SignedBlock> {
    /// Get header.
    #[method(name = "chain_getHeader")]
    fn header(&self, hash: Option<Hash>) -> RpcResult<Option<Header>>;

    /// Get header and body of a relay chain block.
    #[method(name = "chain_getBlock")]
    fn block(&self, hash: Option<NumberOrHex>) -> RpcResult<Option<SignedBlock>>;

    /// Get hash of the n-th block in the canon chain.
    ///
    /// By default returns latest block hash.
    #[method(name = "chain_getBlockHash", aliases = ["chain_getHead"])]
    fn block_hash(&self, hash: Option<NumberOrHex>) -> RpcResult<Option<Hash>>;

    /// Returns the runtime metadata as an opaque blob.
    #[method(name = "state_getMetadata")]
    fn metadata(&self, hash: Option<Hash>) -> RpcResult<String>;

    /// Get the runtime version.
    #[method(name = "state_getRuntimeVersion", aliases = ["chain_getRuntimeVersion"])]
    fn runtime_version(&self, hash: Option<NumberOrHex>) -> RpcResult<RuntimeVersion>;

    /// Returns a storage entry at a specific block's state.
    #[method(name = "state_getStorage", aliases = ["state_getStorageAt"], blocking)]
    fn storage(&self, key: StorageKey, hash: Option<Hash>) -> RpcResult<Option<StorageData>>;

    /// Query storage entries (by key) starting at block hash given as the second parameter.
    #[method(name = "state_queryStorageAt", blocking)]
    fn query_storage_at(
        &self,
        keys: Vec<StorageKey>,
        at: Option<Hash>,
    ) -> RpcResult<Vec<StorageChangeSet<Hash>>>;

    /// Get the node's implementation name. Plain old string.
    #[method(name = "system_name")]
    fn system_name(&self) -> RpcResult<String>;

    #[method(name = "system_version")]
    fn system_version(&self) -> RpcResult<String>;

    /// Get the chain's name. Given as a string identifier.
    #[method(name = "system_chain")]
    fn system_chain(&self) -> RpcResult<String>;

    /// Get the chain's type.
    #[method(name = "system_chainType")]
    fn system_type(&self) -> RpcResult<ChainType>;

    /// Get a custom set of properties as a JSON object, defined in the chain spec.
    #[method(name = "system_properties")]
    fn system_properties(&self) -> RpcResult<Properties>;

    /// Author -------------------------------------------------------------------------------------
    /// Submit hex-encoded extrinsic for inclusion in block.
    #[method(name = "author_submitExtrinsic")]
    fn submit_extrinsic(&self, extrinsic: Bytes) -> RpcResult<Hash>;

    /// Submit an extrinsic to watch.
    #[subscription(
        name = "author_submitAndWatchExtrinsic" => "author_extrinsicUpdate",
        unsubscribe = "author_unwatchExtrinsic",
        item = TransactionStatus<Hash, BlockHash>,
    )]
    fn watch_extrinsic(&self, extrinsic: Bytes);
    /// --------------------------------------------------------------------------------------------

    #[method(name = "rpc_methods")]
    fn methods(&self) -> RpcResult<RpcMethods>;

    /// Get hash of the last finalized block in the canon chain.
    #[method(name = "chain_getFinalizedHead", aliases = ["chain_getFinalisedHead"])]
    fn finalized_head(&self) -> RpcResult<Hash>;

    /// New runtime version subscription
    #[subscription(
        name = "state_subscribeRuntimeVersion" => "state_runtimeVersion",
        unsubscribe = "state_unsubscribeRuntimeVersion",
        aliases = ["chain_subscribeRuntimeVersion"],
        unsubscribe_aliases = ["chain_unsubscribeRuntimeVersion"],
        item = RuntimeVersion,
    )]
    fn subscribe_runtime_version(&self);

    /// New head subscription.
    #[subscription(
        name = "chain_subscribeNewHeads" => "chain_newHead",
        aliases = ["subscribe_newHead", "chain_subscribeNewHead"],
        unsubscribe = "chain_unsubscribeNewHeads",
        unsubscribe_aliases = ["unsubscribe_newHead", "chain_unsubscribeNewHead"],
        item = Header
    )]
    fn subscribe_new_heads(&self);

    /// Finalized head subscription.
    #[subscription(
        name = "chain_subscribeFinalizedHeads" => "chain_finalizedHead",
        aliases = ["chain_subscribeFinalisedHeads"],
        unsubscribe = "chain_unsubscribeFinalizedHeads",
        unsubscribe_aliases = ["chain_unsubscribeFinalisedHeads"],
        item = Header
    )]
    fn subscribe_finalized_heads(&self);

    /// New storage subscription
    #[subscription(
        name = "state_subscribeStorage" => "state_storage",
        unsubscribe = "state_unsubscribeStorage",
        item = StorageChangeSet<Hash>,
    )]
    fn subscribe_storage(&self, keys: Option<Vec<StorageKey>>);

    /// System
    #[method(name = "system_accountNextIndex", aliases = ["account_nextIndex"])]
    async fn nonce(&self, account: AccountId) -> RpcResult<Index>;

    /// Tweaks
    /// Get last Header.
    #[method(name = "tweaks_last_header")]
    fn tweaks_last_header(&self) -> RpcResult<Header>;

    /// DEBUG
    #[method(name = "debug_setBlockNumber")]
    fn set_block_number(&self, number: NumberOrHex) -> RpcResult<()>;
}

pub struct MockRpcServer {
    pub db: Arc<Mutex<Database>>,
}

impl MockRpcServer {
    pub fn new(db: Arc<Mutex<Database>>) -> Self {
        Self { db }
    }
}

fn subscribe_storage(
    backend: InMemoryBackend<Blake2Hasher>,
    notifications: &StorageNotifications<Block>,
    mut sink: SubscriptionSink,
    keys: Option<Vec<StorageKey>>,
) {
    let _ = sink.accept();
    println!("subscribe_storage(id={:?})", sink.subscription_id());

    let stream = notifications.listen(None, None);

    // initial values
    let initial = stream::iter(keys.map(|keys| {
        let changes = keys
            .into_iter()
            .map(|key| {
                let k = &hex::decode(&key[2..]).unwrap_or_default()[..];
                let value = match backend.storage(k) {
                    Ok(Some(v)) => Some(format!("0x{}", hex::encode(v))),
                    _ => None,
                };
                (key, value)
            })
            .collect();
        StorageChangeSet {
            block: Default::default(),
            changes,
        }
    }));

    let storage_stream = stream.map(move |n| StorageChangeSet {
        block: n.block,
        changes: n
            .changes
            .iter()
            .filter_map(|(o_sk, k, v)| {
                o_sk.is_none().then(|| {
                    let key = format!("0x{}", hex::encode(k));
                    let value = v.map(|v| format!("0x{}", hex::encode(&v.0[..])));
                    (key, value)
                })
            })
            .collect(),
    });

    let stream = initial
        .chain(storage_stream)
        .filter(|storage| future::ready(!storage.changes.is_empty()));

    let fut = async move {
        sink.pipe_from_stream(stream).await;
    };

    tokio::spawn(fut.boxed());
}

#[allow(dead_code)]
async fn send(method: &str, _params: Option<String>) -> anyhow::Result<Option<Header>> {
    let url = format!("ws://{}", ENDPOINT);
    let client1 = WsClientBuilder::default().build(&url).await?;
    let response: Option<Header> = client1.request(method, rpc_params![]).await?;
    Ok(response)
}

fn sign_block(block: Block) -> SignedBlock {
    SignedBlock::new(block, None)
}

#[async_trait]
impl MockApiServer<AccountId, Number, Hash, Header, BlockHash, SignedBlock> for MockRpcServer {
    fn header(&self, hash: Option<Hash>) -> RpcResult<Option<Header>> {
        println!("----> header(hash={:?})", hash.clone());
        let db = self.db.lock().unwrap();
        let h = match hash {
            Some(h) => h,
            None => db.head.hash(),
        };
        let block = match db.blocks.get(&h) {
            Some(s) => s.block.clone(),
            None => return Ok(None),
        };
        Ok(Some(block.header))
    }

    fn block(&self, hash: Option<NumberOrHex>) -> RpcResult<Option<SignedBlock>> {
        println!("----> block(hash={:?})", hash.clone());
        let db = self.db.lock().unwrap();
        let h = match hash {
            Some(hash) => match hash {
                NumberOrHex::Number(n) => match db.num_to_hash.get(&n) {
                    Some(h) => *h,
                    None => return Ok(None),
                },
                NumberOrHex::Hex(h) => h,
            },
            None => db.head.hash(),
        };

        let block = match db.blocks.get(&h) {
            Some(s) => s.block.clone(),
            None => return Ok(None),
        };

        Ok(Some(sign_block(block)))
    }

    fn block_hash(&self, hash: Option<NumberOrHex>) -> RpcResult<Option<Hash>> {
        println!("----> block_hash(hash={:?})", hash.clone());
        let db = self.db.lock().unwrap();
        let h = match hash {
            Some(hash) => match hash {
                NumberOrHex::Number(n) => match db.num_to_hash.get(&n) {
                    Some(h) => *h,
                    None => return Ok(None),
                },
                NumberOrHex::Hex(h) => h,
            },
            None => db.head.hash(),
        };
        let block = match db.blocks.get(&h) {
            Some(s) => s.block.clone(),
            None => return Ok(None),
        };
        println!(
            "----> block_hash(hash={:?}) -> {:?}",
            hash.clone(),
            block.hash()
        );
        Ok(Some(block.hash()))
    }

    fn metadata(&self, hash: Option<Hash>) -> RpcResult<String> {
        println!("----> metadata(hash={:?})", hash);
        let m: Vec<u8> = Runtime::metadata().into();
        Ok(format!("0x{}", hex::encode(m)))
    }

    fn runtime_version(&self, hash: Option<NumberOrHex>) -> RpcResult<RuntimeVersion> {
        println!("----> runtime_version(hash={:?})", hash);
        Ok(RuntimeVersion {
            spec_name: "runstrate-node".to_string(),
            impl_name: "runstrate-node".to_string(),
            authoring_version: 1,
            spec_version: 1,
            impl_version: 1,
            apis: vec![],
            transaction_version: 1,
            state_version: 1,
        })
    }

    fn storage(&self, key: StorageKey, hash: Option<Hash>) -> RpcResult<Option<StorageData>> {
        let db = self.db.lock().unwrap();
        let k = &hex::decode(&key[2..]).unwrap_or_default()[..];
        match hash {
            Some(h) => {
                let value = match db.blocks.get(&h) {
                    Some(s) => match s.backend.storage(k) {
                        Ok(Some(v)) => Some(format!("0x{}", hex::encode(v))),
                        _ => return Ok(None),
                    },
                    None => return Ok(None),
                };
                println!(
                    "----> storage(key={:?}, hash={:?}) -> {:?}",
                    &key, hash, value
                );
                Ok(value)
            }
            None => {
                let value = match db.externalities.as_backend().storage(k) {
                    Ok(Some(v)) => Some(format!("0x{}", hex::encode(v))),
                    _ => None,
                };
                println!(
                    "----> storage(key={:?}, hash={:?}) -> {:?}",
                    &key, hash, value
                );
                Ok(value)
            }
        }
    }

    fn query_storage_at(
        &self,
        keys: Vec<StorageKey>,
        at: Option<Hash>,
    ) -> RpcResult<Vec<StorageChangeSet<Hash>>> {
        println!(
            "----> query_storage_at(keys={:?}, at={:?})",
            keys,
            at.clone()
        );
        let db = self.db.lock().unwrap();
        let mut ret = vec![];
        match at {
            None => {
                for key in keys.into_iter() {
                    let k = &hex::decode(&key[2..]).unwrap_or_default()[..];
                    let value = match db.externalities.as_backend().storage(k) {
                        Ok(Some(v)) => Some(format!("0x{}", hex::encode(v))),
                        _ => None,
                    };
                    ret.push(StorageChangeSet {
                        block: db.head.hash(),
                        changes: vec![(key, value)],
                    });
                }
            }
            Some(at) => {
                match db.blocks.get(&at) {
                    Some(s) => {
                        for key in keys.into_iter() {
                            let k = &hex::decode(&key[2..]).unwrap_or_default()[..];
                            let value = match s.backend.storage(k) {
                                Ok(Some(v)) => Some(format!("0x{}", hex::encode(v))),
                                _ => None,
                            };
                            ret.push(StorageChangeSet {
                                block: at,
                                changes: vec![(key, value)],
                            });
                        }
                    }
                    None => return Ok(ret),
                };
            }
        };
        Ok(ret)
    }

    fn finalized_head(&self) -> RpcResult<Hash> {
        println!("----> finalized_head()");
        let db = self.db.lock().unwrap();
        Ok(db.head.hash())
    }

    fn subscribe_runtime_version(&self, mut sink: SubscriptionSink) -> SubscriptionResult {
        println!("----> subscribe_runtime_version()");
        let _ = sink.accept();
        Ok(())
    }

    fn subscribe_new_heads(&self, mut sink: SubscriptionSink) -> SubscriptionResult {
        let _ = sink.accept();
        let sub_id = sink.subscription_id();
        println!("----> subscribe_new_heads(id={:?})", sub_id);
        let db_ref = self.db.clone();
        thread::spawn(move || -> anyhow::Result<()> {
            loop {
                // TODO: Not good...channels?
                let db = db_ref.lock().unwrap();
                sink.send(db.head.header())?;
                drop(db);
                if sink.is_closed() {
                    break;
                };
                thread::sleep(time::Duration::from_millis(100));
            }
            Ok(())
        });
        Ok(())
    }

    fn subscribe_finalized_heads(&self, mut sink: SubscriptionSink) -> SubscriptionResult {
        println!("----> subscribe_finalized_heads()");
        let _ = sink.accept();
        Ok(())
    }

    fn subscribe_storage(
        &self,
        sink: SubscriptionSink,
        keys: Option<Vec<StorageKey>>,
    ) -> SubscriptionResult {
        println!("----> subscribe_storage(keys={:?})", keys);
        let mut db = self.db.lock().unwrap();
        let backend = db.externalities.as_backend();
        subscribe_storage(backend, &db.notifications, sink, keys.clone());
        if let Some(keys) = keys {
            for k in keys {
                if !db.subs_storage_key.iter().any(|each| *each == k) {
                    db.subs_storage_key.push(k);
                }
            }
        };
        println!(
            "----> subscribe_storage(subs_storage_key={:?})",
            db.subs_storage_key.len()
        );
        println!(
            "----> subscribe_storage(subs_storage_key={:?})",
            db.subs_storage_key
        );
        Ok(())
    }

    fn system_name(&self) -> RpcResult<String> {
        println!("----> system_name()");
        Ok("Runstrate".to_string())
    }

    fn system_version(&self) -> RpcResult<String> {
        println!("----> system_version()");
        Ok("v0.0.1".to_string())
    }

    fn system_chain(&self) -> RpcResult<String> {
        println!("----> system_chain()");
        Ok("dev".to_string())
    }

    fn system_type(&self) -> RpcResult<ChainType> {
        println!("----> system_type()");
        Ok(ChainType::Development)
    }

    fn system_properties(&self) -> RpcResult<Properties> {
        println!("----> system_properties()");
        Ok(Default::default())
    }

    fn submit_extrinsic(&self, extrinsic: Bytes) -> RpcResult<Hash> {
        println!("----> submit_extrinsic()");
        let mut db = self.db.lock().unwrap();
        db.pool.push(extrinsic.clone());
        let ext = hex::decode(extrinsic).unwrap_or_default();
        Ok(blake2_256(&ext).into())
    }

    fn watch_extrinsic(&self, mut sink: SubscriptionSink, extrinsic: Bytes) -> SubscriptionResult {
        println!("----> watch_extrinsic(extrinsic={:?})", extrinsic);
        let _ = sink.accept();
        let mut db = self.db.lock().unwrap();
        let xt_hash = H256::from_slice(&blake2_256(extrinsic.as_bytes()));
        db.pool.push(extrinsic);
        db.extrinsics_status
            .insert(xt_hash, TransactionStatus::Ready);
        let db_ref = self.db.clone();
        thread::spawn(move || -> anyhow::Result<()> {
            loop {
                // TODO: Not good...channels?
                let db = db_ref.lock().unwrap();
                match db.extrinsics_status.get(&xt_hash) {
                    Some(status) => {
                        sink.send(status)?;
                        match status {
                            TransactionStatus::Ready => (),
                            _ => break,
                        }
                    }
                    _ => break,
                };
                drop(db);
                if sink.is_closed() {
                    break;
                };
                thread::sleep(time::Duration::from_millis(100));
            }
            Ok(())
        });
        Ok(())
    }

    fn methods(&self) -> RpcResult<RpcMethods> {
        println!("----> methods()");
        Ok(RpcMethods::Unsafe)
    }

    /// System
    async fn nonce(&self, account: AccountId) -> RpcResult<Index> {
        println!("----> nonce(account={:?})", account);
        let mut nonce = 0;
        let mut db = self.db.lock().unwrap();
        let _ = db.externalities.execute_with(|| -> anyhow::Result<()> {
            nonce = crate::System::account_nonce(account);
            Ok(())
        });
        Ok(nonce)
    }

    /// Tweaks
    fn tweaks_last_header(&self) -> RpcResult<Header> {
        let db = self.db.lock().unwrap();
        Ok(db.head.clone().header)
    }

    /// DEBUG
    fn set_block_number(&self, number: NumberOrHex) -> RpcResult<()> {
        println!("----> set_block_number(number={:?})", number);
        Ok(())
    }
}

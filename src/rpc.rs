use jsonrpsee::{
    core::{async_trait, server::rpc_module::SubscriptionSink, RpcResult},
    proc_macros::rpc,
    types::SubscriptionResult,
};

use futures::{future, stream, FutureExt, StreamExt};
use rand::Rng;
use std::{
    sync::{Arc, Mutex},
    thread, time,
};

use sc_client_api::StorageNotifications;
use sp_state_machine::{Backend, InMemoryBackend};

use sp_core::{blake2_256, Blake2Hasher, H256};
use sp_runtime::traits::Block as BlockT;

use crate::{
    account::AccountId,
    rpc_types::{
        BlockHash, Bytes, ChainType, Hash, Header, Index, Number, NumberOrHex, Properties,
        RpcMethods, RuntimeVersion, SignedBlock, StorageChangeSet, StorageData, StorageKey,
        TransactionStatus,
    },
    Block, Database, Runtime,
};

pub struct MockRpcServer {
    pub db: Arc<Mutex<Database>>,
    pub runtime_version: RuntimeVersion,
}

impl MockRpcServer {
    pub fn new(db: Arc<Mutex<Database>>) -> Self {
        // Generating a random version (on every build)
        // to force UIs to refresh on their end too.
        let r = rand::thread_rng().gen_range(0..1000);
        Self {
            db,
            runtime_version: RuntimeVersion {
                spec_name: "runstrate-node".to_string(),
                impl_name: "runstrate-node".to_string(),
                authoring_version: r,
                spec_version: r,
                impl_version: r,
                apis: vec![],
                transaction_version: r,
                state_version: 1,
            },
        }
    }

    async fn header(&self, hash: Option<Hash>) -> RpcResult<Option<Header>> {
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

    async fn block(&self, hash: Option<NumberOrHex>) -> RpcResult<Option<SignedBlock>> {
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

        Ok(Some(SignedBlock::new(block, None)))
    }

    async fn block_hash(&self, hash: Option<NumberOrHex>) -> RpcResult<Option<Hash>> {
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
        Ok(Some(block.hash()))
    }

    async fn metadata(&self, _hash: Option<Hash>) -> RpcResult<String> {
        let m: Vec<u8> = Runtime::metadata().into();
        Ok(format!("0x{}", hex::encode(m)))
    }

    async fn runtime_version(&self, _hash: Option<NumberOrHex>) -> RpcResult<RuntimeVersion> {
        Ok(self.runtime_version.clone())
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
                Ok(value)
            }
            None => {
                let value = match db.externalities.as_backend().storage(k) {
                    Ok(Some(v)) => Some(format!("0x{}", hex::encode(v))),
                    _ => None,
                };
                Ok(value)
            }
        }
    }

    fn query_storage_at(
        &self,
        keys: Vec<StorageKey>,
        at: Option<Hash>,
    ) -> RpcResult<Vec<StorageChangeSet<Hash>>> {
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

    async fn finalized_head(&self) -> RpcResult<Hash> {
        let db = self.db.lock().unwrap();
        Ok(db.head.hash())
    }

    async fn system_name(&self) -> RpcResult<String> {
        Ok("Runstrate".to_string())
    }

    async fn system_version(&self) -> RpcResult<String> {
        Ok("v0.0.2".to_string())
    }

    async fn system_chain(&self) -> RpcResult<String> {
        Ok("dev".to_string())
    }

    async fn system_type(&self) -> RpcResult<ChainType> {
        Ok(ChainType::Development)
    }

    async fn system_properties(&self) -> RpcResult<Properties> {
        Ok(Default::default())
    }

    async fn submit_extrinsic(&self, extrinsic: Bytes) -> RpcResult<Hash> {
        let mut db = self.db.lock().unwrap();
        db.pool.push(extrinsic.clone());
        let ext = hex::decode(extrinsic).unwrap_or_default();
        Ok(blake2_256(&ext).into())
    }

    async fn methods(&self) -> RpcResult<RpcMethods> {
        Ok(RpcMethods::Unsafe)
    }

    /// System
    async fn nonce(&self, account: AccountId) -> RpcResult<Index> {
        let mut nonce = 0;
        let mut db = self.db.lock().unwrap();
        let _ = db.externalities.execute_with(|| -> anyhow::Result<()> {
            nonce = crate::System::account_nonce(account);
            Ok(())
        });
        Ok(nonce)
    }
}

#[rpc(server)]
pub trait MockApi<AccountId, Number, Hash, Header, BlockHash, SignedBlock> {
    /// Get header.
    #[method(name = "chain_getHeader")]
    async fn header(&self, hash: Option<Hash>) -> RpcResult<Option<Header>>;

    /// Get header and body of a relay chain block.
    #[method(name = "chain_getBlock")]
    async fn block(&self, hash: Option<NumberOrHex>) -> RpcResult<Option<SignedBlock>>;

    /// Get hash of the n-th block in the canon chain.
    ///
    /// By default returns latest block hash.
    #[method(name = "chain_getBlockHash", aliases = ["chain_getHead"])]
    async fn block_hash(&self, hash: Option<NumberOrHex>) -> RpcResult<Option<Hash>>;

    /// Returns the runtime metadata as an opaque blob.
    #[method(name = "state_getMetadata")]
    async fn metadata(&self, hash: Option<Hash>) -> RpcResult<String>;

    /// Get the runtime version.
    #[method(name = "state_getRuntimeVersion", aliases = ["chain_getRuntimeVersion"])]
    async fn runtime_version(&self, hash: Option<NumberOrHex>) -> RpcResult<RuntimeVersion>;

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
    async fn system_name(&self) -> RpcResult<String>;

    #[method(name = "system_version")]
    async fn system_version(&self) -> RpcResult<String>;

    /// Get the chain's name. Given as a string identifier.
    #[method(name = "system_chain")]
    async fn system_chain(&self) -> RpcResult<String>;

    /// Get the chain's type.
    #[method(name = "system_chainType")]
    async fn system_type(&self) -> RpcResult<ChainType>;

    /// Get a custom set of properties as a JSON object, defined in the chain spec.
    #[method(name = "system_properties")]
    async fn system_properties(&self) -> RpcResult<Properties>;

    /// Author -------------------------------------------------------------------------------------
    /// Submit hex-encoded extrinsic for inclusion in block.
    #[method(name = "author_submitExtrinsic")]
    async fn submit_extrinsic(&self, extrinsic: Bytes) -> RpcResult<Hash>;

    /// Submit an extrinsic to watch.
    #[subscription(
        name = "author_submitAndWatchExtrinsic" => "author_extrinsicUpdate",
        unsubscribe = "author_unwatchExtrinsic",
        item = TransactionStatus<Hash, BlockHash>,
    )]
    fn watch_extrinsic(&self, extrinsic: Bytes);
    /// --------------------------------------------------------------------------------------------

    #[method(name = "rpc_methods")]
    async fn methods(&self) -> RpcResult<RpcMethods>;

    /// Get hash of the last finalized block in the canon chain.
    #[method(name = "chain_getFinalizedHead", aliases = ["chain_getFinalisedHead"])]
    async fn finalized_head(&self) -> RpcResult<Hash>;

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
}

#[async_trait]
impl MockApiServer<AccountId, Number, Hash, Header, BlockHash, SignedBlock> for MockRpcServer {
    async fn header(&self, hash: Option<Hash>) -> RpcResult<Option<Header>> {
        println!("----> header(hash={:?})", hash.clone());
        self.header(hash).await
    }
    async fn block(&self, hash: Option<NumberOrHex>) -> RpcResult<Option<SignedBlock>> {
        println!("----> block(hash={:?})", hash.clone());
        self.block(hash).await
    }
    async fn block_hash(&self, hash: Option<NumberOrHex>) -> RpcResult<Option<Hash>> {
        println!("----> block_hash(hash={:?})", hash.clone());
        self.block_hash(hash).await
    }
    async fn metadata(&self, hash: Option<Hash>) -> RpcResult<String> {
        println!("----> metadata(hash={:?})", hash);
        self.metadata(hash).await
    }
    async fn runtime_version(&self, hash: Option<NumberOrHex>) -> RpcResult<RuntimeVersion> {
        println!("----> runtime_version(hash={:?})", hash);
        self.runtime_version(hash).await
    }
    fn storage(&self, key: StorageKey, hash: Option<Hash>) -> RpcResult<Option<StorageData>> {
        println!("----> storage(key={:?}, hash={:?})", &key, hash);
        self.storage(key, hash)
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
        self.query_storage_at(keys, at)
    }
    async fn finalized_head(&self) -> RpcResult<Hash> {
        println!("----> finalized_head()");
        self.finalized_head().await
    }
    fn subscribe_runtime_version(&self, mut sink: SubscriptionSink) -> SubscriptionResult {
        println!("----> subscribe_runtime_version()");
        let _ = sink.accept();
        Ok(())
    }

    fn subscribe_new_heads(&self, mut sink: SubscriptionSink) -> SubscriptionResult {
        println!("----> subscribe_new_heads()");
        let _ = sink.accept();
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
        Ok(())
    }
    async fn system_name(&self) -> RpcResult<String> {
        println!("----> system_name()");
        self.system_name().await
    }
    async fn system_version(&self) -> RpcResult<String> {
        println!("----> system_version()");
        self.system_version().await
    }
    async fn system_chain(&self) -> RpcResult<String> {
        println!("----> system_chain()");
        self.system_chain().await
    }
    async fn system_type(&self) -> RpcResult<ChainType> {
        println!("----> system_type()");
        self.system_type().await
    }
    async fn system_properties(&self) -> RpcResult<Properties> {
        println!("----> system_properties()");
        self.system_properties().await
    }
    async fn submit_extrinsic(&self, extrinsic: Bytes) -> RpcResult<Hash> {
        println!("----> submit_extrinsic()");
        self.submit_extrinsic(extrinsic).await
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
    async fn methods(&self) -> RpcResult<RpcMethods> {
        println!("----> methods()");
        self.methods().await
    }
    /// System
    async fn nonce(&self, account: AccountId) -> RpcResult<Index> {
        println!("----> nonce(account={:?})", account);
        self.nonce(account).await
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

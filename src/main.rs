mod account;
mod externalities;
mod mock;
mod rpc;
mod rpc_types;

/// Add runtime here -------------------------------------------
use mock::{
    Address, Block as TargetBlock, Header, Runtime as TargetRuntime, RuntimeOrigin, System,
    UncheckedExtrinsic,
};

// use kusama_runtime::{
//     Address, Block as TargetBlock, Header, Runtime as TargetRuntime, RuntimeOrigin,
//     System, UncheckedExtrinsic,
// };

use std::time::SystemTime;
use std::{
    collections::HashMap,
    default::Default,
    sync::{Arc, Mutex},
    thread, time,
};

use jsonrpsee::{server::ServerBuilder, RpcModule};

use codec::Decode;
use frame_support::dispatch::{DispatchResultWithPostInfo, GetDispatchInfo};
use pallet_timestamp::Now;
use sc_client_api::StorageNotifications;
use sp_core::{blake2_256, Blake2Hasher, Encode, H256};
use sp_runtime::{
    traits::{Block as BlockT, Dispatchable, Header as HeaderT, SignedExtension},
    DispatchError, SaturatedConversion,
};
use sp_state_machine::{Backend, InMemoryBackend};

use pallet_transaction_payment::ChargeTransactionPayment;

use crate::account::{get_account_id, AccountId};
use crate::externalities::new_test_ext;
use crate::rpc::{MockApiServer, MockRpcServer};
use crate::rpc_types::{BlockHash, BlockNumber, StorageKey, TransactionStatus};

pub type Runtime = TargetRuntime;
pub type Block = TargetBlock;
pub type Extrinsic = UncheckedExtrinsic;
pub type Context = frame_system::ChainContext<TargetRuntime>;

pub const MILLISECS_PER_BLOCK: u64 = 6000;

pub const ENDPOINT: &str = "127.0.0.1:9944";
pub const MEGABYTE: u32 = 1025 * 1024;

pub type ExtrinsicHashAndStatus = (Vec<(H256, Extrinsic)>, Vec<(H256, ())>);

pub struct BlockChainData {
    pub head: TargetBlock,
    pub blocks: HashMap<BlockHash, StorageAt>,
    pub num_to_hash: HashMap<BlockNumber, BlockHash>,
    pub pool: Vec<String>,
    pub extrinsics_status: HashMap<H256, TransactionStatus<H256, BlockHash>>,
    pub subs_storage_key: Vec<StorageKey>,
    pub notifications: StorageNotifications<Block>,
}

pub struct StorageAt {
    pub block: TargetBlock,
    pub backend: InMemoryBackend<Blake2Hasher>,
}

pub struct Database {
    pub inner: BlockChainData,
}

impl std::ops::Deref for Database {
    type Target = BlockChainData;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for Database {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

async fn run_server(db: Arc<Mutex<Database>>) -> anyhow::Result<()> {
    let server = ServerBuilder::new()
        .max_request_body_size(15 * MEGABYTE)
        .max_response_body_size(15 * MEGABYTE)
        .max_connections(100)
        .max_subscriptions_per_connection(1024)
        .ping_interval(time::Duration::from_secs(30))
        .build(ENDPOINT)
        .await?;
    let mut module = RpcModule::new(());
    module.merge(MockRpcServer::new(db).into_rpc())?;
    let addr = server.local_addr()?;
    let handle = server.start(module)?;
    println!("Listening on port ws://{}", addr);
    handle.stopped().await;
    Ok(())
}

fn hex_to_xt(bytes: String) -> Result<Extrinsic, ()> {
    let ext = hex::decode(&bytes[2..]).expect("Cannot decode extrinsic data.");
    let extrinsic = match Extrinsic::decode(&mut &ext[..]) {
        Ok(c) => c,
        Err(_) => return Err(()),
    };
    Ok(extrinsic)
}

fn check_pending_extrinsics(extrinsics: Vec<String>) -> ExtrinsicHashAndStatus {
    let mut pending_extrinsics = (vec![], vec![]);
    for xt in extrinsics {
        let xt_hash = H256::from_slice(&blake2_256(xt.as_bytes()));
        match hex_to_xt(xt) {
            Ok(xt) => pending_extrinsics.0.push((xt_hash, xt)),
            _ => pending_extrinsics.1.push((xt_hash, ())),
        }
    }
    pending_extrinsics
}

fn charge_fees_and_dispatch(account: &AccountId, uxt: Extrinsic) -> DispatchResultWithPostInfo {
    let encoded_len = uxt.encode().len();
    let dispatch_info = uxt.get_dispatch_info();
    let pre = ChargeTransactionPayment::<Runtime>::from(0)
        .pre_dispatch(account, &uxt.function, &dispatch_info, encoded_len)
        .map_err(|e| DispatchError::Other(e.into()))?;
    let d = uxt
        .function
        .dispatch(RuntimeOrigin::signed(account.clone()))?;
    ChargeTransactionPayment::<Runtime>::post_dispatch(
        Some(pre),
        &dispatch_info,
        &d,
        encoded_len,
        &Ok(()),
    )
    .map_err(|e| DispatchError::Other(e.into()))?;
    Ok(d)
}

fn current_time() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .saturated_into()
}

fn build_block(header: Header, extrinsics: Vec<Extrinsic>) -> TargetBlock {
    TargetBlock::new(header, extrinsics)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("---- Runstrate ----");

    let mut block_number = 0;

    // Genesis Block
    let mut block = build_block(
        Header::new(
            block_number,
            [1u8; 32].into(),
            [1u8; 32].into(),
            Default::default(),
            Default::default(),
        ),
        vec![],
    );

    let mut externalities = new_test_ext();

    let storage = StorageAt {
        block: block.clone(),
        backend: externalities.as_backend(),
    };

    let mutex_db = Arc::new(Mutex::new(Database {
        inner: BlockChainData {
            head: block.clone(),
            blocks: HashMap::from([(block.hash(), storage)]),
            num_to_hash: HashMap::from([(block_number, block.hash())]),
            pool: vec![],
            extrinsics_status: HashMap::new(),
            subs_storage_key: vec![],
            notifications: StorageNotifications::new(None),
        },
    }));

    println!("GenesisBlock: {:04} ({:?})", block_number, block.hash());

    tokio::spawn(run_server(mutex_db.clone()));

    loop {
        block_number += 1;
        let mut header = block.clone().header;

        let mut db = mutex_db.lock().unwrap();
        let (extrinsics, invalid) = check_pending_extrinsics(db.pool.clone());
        let mut extrinsics_status: Vec<_> = invalid
            .iter()
            .map(|h| (h.0, TransactionStatus::Invalid))
            .collect();

        let _ = externalities.execute_with(|| -> anyhow::Result<()> {
            // Manually resetting Events.
            System::reset_events();

            System::initialize(&block_number, &block.hash(), header.digest());
            System::note_finished_initialize();

            // Forcing pallet_timestamp set()
            Now::<Runtime>::put(current_time());

            if !extrinsics.is_empty() {
                println!("Extrinsics : {:?}", extrinsics.clone());
                for (idx, (xt_hash, uxt)) in extrinsics.clone().into_iter().enumerate() {
                    let encoded = uxt.encode();
                    System::note_extrinsic(encoded);
                    let dispatch_info = uxt.get_dispatch_info();

                    let signer = match uxt.clone().signature {
                        Some((address, _, _)) => match address {
                            Address::Id(a) => Some(a),
                            Address::Address32(a) => Some(a.into()),
                            _ => Some(get_account_id("//Alice")),
                        },
                        _ => None,
                    };

                    let r = match &signer {
                        Some(s) => {
                            System::inc_account_nonce(s.clone());
                            charge_fees_and_dispatch(s, uxt.clone())
                        }
                        None => uxt.function.dispatch(RuntimeOrigin::none()),
                    };

                    System::note_applied_extrinsic(&r, dispatch_info);

                    extrinsics_status.push((
                        xt_hash,
                        TransactionStatus::InBlock((Default::default(), idx)),
                    ));

                    println!("Extrinsic->: (signer={:?} res={:?})", signer, r);
                    println!("Events     : {:?}", System::events());
                }
            }

            System::note_finished_extrinsics();
            header = System::finalize();

            Ok(())
        });

        db.pool = vec![];

        block = build_block(header, extrinsics.into_iter().map(|x| x.1).collect());
        let storage = StorageAt {
            block: block.clone(),
            backend: externalities.as_backend(),
        };
        db.blocks.insert(block.hash(), storage);
        db.num_to_hash.insert(block_number, block.hash());
        db.head = block.clone();

        // Update extrinsics status for Watchers
        for (h, s) in extrinsics_status.into_iter() {
            let s = match s {
                TransactionStatus::InBlock((_, i)) => TransactionStatus::InBlock((block.hash(), i)),
                _ => s,
            };
            println!("Extrinsic(h, s): {:?} -> {:?}", h, s);
            db.extrinsics_status.insert(h, s);
        }

        // Notify storage subscribers
        let mut changeset = vec![];
        let c_changeset_1 = vec![(vec![5], Some(vec![4])), (vec![6], None)];
        let c_changeset = vec![(vec![4], c_changeset_1)];
        for storage_key in &db.subs_storage_key {
            let k = &hex::decode(&storage_key[2..]).unwrap_or_default()[..];
            let value: Option<Vec<u8>> = match externalities.as_backend().storage(k) {
                Ok(Some(v)) => Some(v),
                _ => None,
            };
            let key: Vec<u8> = hex::decode(&storage_key[2..]).unwrap_or_default()[..].into();
            changeset.push((key, value));
        }

        db.notifications.trigger(
            &block.hash(),
            changeset.into_iter(),
            c_changeset.into_iter().map(|(a, b)| (a, b.into_iter())),
        );

        drop(db);

        println!("BlockNumber: {:04} ({:?})", block_number, block.hash());

        // Giving some time to RPCs be processed.
        thread::sleep(time::Duration::from_millis(MILLISECS_PER_BLOCK));
    }
}

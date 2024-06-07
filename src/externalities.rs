use crate::{get_account_id};

use sp_runtime::BuildStorage;
use sp_storage::Storage;
use polkadot_primitives::AccountId;

// use mock_runtime::{BalancesConfig, SudoConfig};

// use solochain_template_runtime::{BalancesConfig, SudoConfig, Runtime};
use kusama_runtime::{BalancesConfig, BuildStorage as KusamaBuildStorage, Runtime};
// use statemine_runtime::BalancesConfig;

fn set_balances(storage: &mut Storage, endowed_accounts: Option<Vec<AccountId>>) {
    let accounts =
        endowed_accounts.unwrap_or(vec![get_account_id("//Alice"), get_account_id("//Bob")]);
    let config = BalancesConfig {
        balances: accounts
            .iter()
            .cloned()
            .map(|k| (k, 1_000_000_000_000_000u128))
            .collect(),
    };
    config.assimilate_storage(storage).unwrap();
}

// Comment this out if not using the pallet_sudo
// fn set_sudo(storage: &mut Storage, account: AccountId) {
//     let config = SudoConfig { key: Some(account) };
//     config.assimilate_storage(storage).unwrap();
// }

/// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    let mut storage = frame_system::GenesisConfig::<Runtime>::default()
        .build_storage()
        .unwrap();

    set_balances(&mut storage, None);

    // Comment this out if using Kusama runtime
    // set_sudo(&mut storage, get_account_id("//Alice"));

    storage.into()
}

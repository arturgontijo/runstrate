use crate::{get_account_id, Runtime};
use sp_core::{crypto::AccountId32, storage::Storage};

use frame_support::traits::GenesisBuild;

use mock_runtime::{BalancesConfig, SudoConfig};

// use kusama_runtime::{BalancesConfig};
// use statemine_runtime::{BalancesConfig};

fn set_balances(storage: &mut Storage, endowed_accounts: Option<Vec<AccountId32>>) {
    let accounts =
        endowed_accounts.unwrap_or(vec![get_account_id("//Alice"), get_account_id("//Bob")]);
    let config = BalancesConfig {
        balances: accounts
            .iter()
            .cloned()
            .map(|k| (k, 1_000_000_000_000_000))
            .collect(),
    };
    config.assimilate_storage(storage).unwrap();
}

// Comment out this if not using the pallet_sudo
fn set_sudo(storage: &mut Storage, account: AccountId32) {
    let config = SudoConfig { key: Some(account) };
    config.assimilate_storage(storage).unwrap();
}

/// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    let mut storage = frame_system::GenesisConfig::default()
        .build_storage::<Runtime>()
        .unwrap();

    set_balances(&mut storage, None);

    // Comment out this if using Kusama runtime
    set_sudo(&mut storage, get_account_id("//Alice"));

    storage.into()
}

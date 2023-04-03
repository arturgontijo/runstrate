use crate::get_account_id;
use sp_runtime::BuildStorage;

use crate::mock::{BalancesConfig, GenesisConfig, SudoConfig, SystemConfig};

/// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    let endowed_accounts = vec![get_account_id("//Alice"), get_account_id("//Bob")];
    let t = GenesisConfig {
        system: SystemConfig { code: vec![] },
        balances: BalancesConfig {
            balances: endowed_accounts
                .iter()
                .cloned()
                .map(|k| (k, 1 << 60))
                .collect(),
        },
        sudo: SudoConfig {
            key: Some(get_account_id("//Alice")),
        },
    }
    .build_storage()
    .unwrap();
    t.into()
}

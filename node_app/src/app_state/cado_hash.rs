use eld_common::account::Account;
use eld_common::staking_account::StakingAccount;

#[derive(Debug, Clone)]
pub struct InstanceWithCadoHash<I> {
    pub instance: I,
    pub hash: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct StakingAccountWithCadoHash {
    pub account: StakingAccount,
    pub hash: [u8; 32],
}

#[derive(Debug, Clone)]
pub struct AccountWithCadoHash {
    pub account: Account,
    pub hash: [u8; 32],
}

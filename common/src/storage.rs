use crate::{account::Account, cado::CadoPath, error::EldError};

pub trait AccountStorage {
    fn get_account_by_path(&self, path: CadoPath) -> Result<Option<Account>, EldError>;
}

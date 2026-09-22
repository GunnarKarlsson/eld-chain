use crate::storage::traits::PinboardQueryStorage;

/// Object-safe storage surface required by the P2P sync coordinator.
pub trait SyncCoordinatorStorage: Send + Sync {
    fn get_pinboard_temp_blob(
        &self,
        content_key: &str,
    ) -> Result<Option<Vec<u8>>, eld_common::error::EldError>;
}

impl<T> SyncCoordinatorStorage for T
where
    T: PinboardQueryStorage + Send + Sync,
{
    fn get_pinboard_temp_blob(
        &self,
        content_key: &str,
    ) -> Result<Option<Vec<u8>>, eld_common::error::EldError> {
        <T as PinboardQueryStorage>::get_pinboard_temp_blob(self, content_key)
    }
}

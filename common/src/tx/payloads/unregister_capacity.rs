use crate::error::EldError;
use crate::tx::parts::{HasAmount, HasSender};
use crate::Address;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[serde(try_from = "UnregisterCapacityTxUnchecked")]
pub struct UnregisterCapacityTx {
    pub sender: Address,  // Address of the sender (capacity provider)
    pub unregister: bool, // Distinguishing field to avoid mixup with other tx types
}

#[derive(Deserialize)]
struct UnregisterCapacityTxUnchecked {
    sender: Address,
    unregister: bool,
}

impl TryFrom<UnregisterCapacityTxUnchecked> for UnregisterCapacityTx {
    type Error = EldError;

    fn try_from(unchecked: UnregisterCapacityTxUnchecked) -> Result<Self, Self::Error> {
        UnregisterCapacityTx::new(unchecked.sender, unchecked.unregister)
    }
}

impl UnregisterCapacityTx {
    pub fn new(sender: Address, unregister: bool) -> Result<Self, EldError> {
        // `unregister` must be true so this payload deserializes as a distinct tx type.
        if !unregister {
            return Err(EldError::ValidationError {
                field: "unregister".to_string(),
                value: unregister.to_string(),
                details: "UnregisterCapacity transaction 'unregister' field must be true"
                    .to_string(),
            });
        }

        Ok(Self { sender, unregister })
    }
}

impl std::fmt::Display for UnregisterCapacityTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "UnregisterCapacityTx {{\n sender: {}\n unregister: {}\n }}",
            self.sender, self.unregister
        )
    }
}

impl HasSender for UnregisterCapacityTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for UnregisterCapacityTx {
    fn amount(&self) -> u128 {
        0 // Capacity unregistration doesn't require an amount
    }
}

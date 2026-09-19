use crate::error::EldError;
use crate::tx::parts::{HasAmount, HasSender};
use crate::Address;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(try_from = "UpdateCapacityMerkleRootTxUnchecked")]
pub struct UpdateCapacityMerkleRootTx {
    pub sender: Address, // Address of the capacity provider
    #[serde(with = "crate::tx::parts::hex_array::hex_vec_u8_32")]
    pub merkle_root: [u8; 32], // New merkle root after content storage
}

#[derive(Deserialize)]
struct UpdateCapacityMerkleRootTxUnchecked {
    sender: Address,
    #[serde(with = "crate::tx::parts::hex_array::hex_vec_u8_32")]
    merkle_root: [u8; 32],
}

impl TryFrom<UpdateCapacityMerkleRootTxUnchecked> for UpdateCapacityMerkleRootTx {
    type Error = EldError;

    fn try_from(unchecked: UpdateCapacityMerkleRootTxUnchecked) -> Result<Self, Self::Error> {
        UpdateCapacityMerkleRootTx::new(unchecked.sender, unchecked.merkle_root)
    }
}

impl UpdateCapacityMerkleRootTx {
    pub fn new(sender: Address, merkle_root: [u8; 32]) -> Result<Self, EldError> {
        Ok(Self {
            sender,
            merkle_root,
        })
    }
}

impl std::fmt::Display for UpdateCapacityMerkleRootTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "UpdateCapacityMerkleRootTx {{\n sender: {}\n merkle_root: {}\n }}",
            self.sender,
            hex::encode(self.merkle_root)
        )
    }
}

impl HasSender for UpdateCapacityMerkleRootTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for UpdateCapacityMerkleRootTx {
    fn amount(&self) -> u128 {
        0 // Merkle root update doesn't require an amount
    }
}

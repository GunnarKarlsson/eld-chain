use crate::error::EldError;
use crate::tx::parts::{HasAmount, HasSender};
use crate::Address;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(try_from = "RegisterCapacityTxUnchecked")]
pub struct RegisterCapacityTx {
    pub sender: Address,     // Address of the sender (capacity provider)
    pub capacity_bytes: u64, // Total capacity in bytes
    #[serde(with = "crate::tx::parts::hex_array::hex_vec_u8_32")]
    pub merkle_root: [u8; 32], // Merkle root of capacity proof
    #[serde(with = "crate::tx::parts::hex_array::hex_vec_u8_32")]
    pub seed: [u8; 32], // Seed for capacity proof
    pub chunk_count: u32,    // Number of chunks in the capacity proof
}

#[derive(Deserialize)]
struct RegisterCapacityTxUnchecked {
    sender: Address,
    capacity_bytes: u64,
    #[serde(with = "crate::tx::parts::hex_array::hex_vec_u8_32")]
    merkle_root: [u8; 32],
    #[serde(with = "crate::tx::parts::hex_array::hex_vec_u8_32")]
    seed: [u8; 32],
    chunk_count: u32,
}

impl TryFrom<RegisterCapacityTxUnchecked> for RegisterCapacityTx {
    type Error = EldError;

    fn try_from(unchecked: RegisterCapacityTxUnchecked) -> Result<Self, Self::Error> {
        RegisterCapacityTx::new(
            unchecked.sender,
            unchecked.capacity_bytes,
            unchecked.merkle_root,
            unchecked.seed,
            unchecked.chunk_count,
        )
    }
}

impl RegisterCapacityTx {
    pub fn new(
        sender: Address,
        capacity_bytes: u64,
        merkle_root: [u8; 32],
        seed: [u8; 32],
        chunk_count: u32,
    ) -> Result<Self, EldError> {
        if capacity_bytes == 0 {
            return Err(EldError::ValidationError {
                field: "capacity_bytes".to_string(),
                value: capacity_bytes.to_string(),
                details: "RegisterCapacity capacity_bytes must be greater than zero".to_string(),
            });
        }
        // `chunk_count` is stored as declared; JSON validation requires it to be >= 1.

        Ok(Self {
            sender,
            capacity_bytes,
            merkle_root,
            seed,
            chunk_count,
        })
    }
}

impl std::fmt::Display for RegisterCapacityTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RegisterCapacityTx {{\n sender: {}\n capacity_bytes: {}\n merkle_root: {}\n seed: {}\n chunk_count: {}\n }}",
            self.sender,
            self.capacity_bytes,
            hex::encode(self.merkle_root),
            hex::encode(self.seed),
            self.chunk_count
        )
    }
}

impl HasSender for RegisterCapacityTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for RegisterCapacityTx {
    fn amount(&self) -> u128 {
        0 // Capacity registration doesn't require an amount
    }
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DeviceStatus {
    Active,
    Inactive,
    Penalized,
    Deregistered,
    Maintenance,     // Device is temporarily unavailable for maintenance
    VerifyingChunks, // Device is currently undergoing chunk verification
    StorageFull,     // Device has reached storage capacity
    LowBandwidth,    // Device is experiencing bandwidth limitations
    Probation,       // Device is under observation due to recent failures
}

impl std::fmt::Display for DeviceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceStatus::Active => write!(f, "Active"),
            DeviceStatus::Inactive => write!(f, "Inactive"),
            DeviceStatus::Penalized => write!(f, "Penalized"),
            DeviceStatus::Deregistered => write!(f, "Deregistered"),
            DeviceStatus::Maintenance => write!(f, "Maintenance"),
            DeviceStatus::VerifyingChunks => write!(f, "VerifyingChunks"),
            DeviceStatus::StorageFull => write!(f, "StorageFull"),
            DeviceStatus::LowBandwidth => write!(f, "LowBandwidth"),
            DeviceStatus::Probation => write!(f, "Probation"),
        }
    }
}

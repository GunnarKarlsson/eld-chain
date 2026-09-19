use crate::coin::Coin;
use crate::error::EldError;
use crate::namespace::{is_reserved_namespace_slug, normalize_namespace_slug};
use crate::tx::parts::{HasAmount, HasSender, TxAmount};
use crate::Address;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[serde(try_from = "AddNamespaceTxUnchecked")]
pub struct AddNamespaceTx {
    pub sender: Address,
    pub namespace_slug: String,
    #[serde(with = "crate::tx::parts::amount::tx_amount_string")]
    pub registration_fee: TxAmount,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddNamespaceTxUnchecked {
    sender: Address,
    namespace_slug: String,
    #[serde(with = "crate::tx::parts::amount::tx_amount_string")]
    registration_fee: TxAmount,
}

impl TryFrom<AddNamespaceTxUnchecked> for AddNamespaceTx {
    type Error = EldError;

    fn try_from(unchecked: AddNamespaceTxUnchecked) -> Result<Self, Self::Error> {
        AddNamespaceTx::new(
            unchecked.sender,
            unchecked.namespace_slug,
            unchecked.registration_fee,
        )
    }
}

impl AddNamespaceTx {
    /// Validates slug rules and fee; stores canonical lowercase `namespace_slug`.
    pub fn new(
        sender: Address,
        namespace_slug: String,
        registration_fee: TxAmount,
    ) -> Result<Self, EldError> {
        let canonical = normalize_namespace_slug(&namespace_slug)?;
        if is_reserved_namespace_slug(&canonical) {
            return Err(EldError::ValidationError {
                field: "namespace_slug".to_string(),
                value: canonical,
                details: "Namespace slug is reserved".to_string(),
            });
        }
        if registration_fee.as_u128() == 0 {
            return Err(EldError::ValidationError {
                field: "registration_fee".to_string(),
                value: registration_fee.to_string(),
                details: "registration_fee must be greater than zero".to_string(),
            });
        }
        Coin::new(registration_fee.into())?;
        Ok(Self {
            sender,
            namespace_slug: canonical,
            registration_fee,
        })
    }
}

impl std::fmt::Display for AddNamespaceTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "AddNamespaceTx {{\n sender: {}\n namespace_slug: {}\n registration_fee: {}\n }}",
            self.sender, self.namespace_slug, self.registration_fee
        )
    }
}

impl HasSender for AddNamespaceTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for AddNamespaceTx {
    fn amount(&self) -> u128 {
        self.registration_fee.as_u128()
    }
}

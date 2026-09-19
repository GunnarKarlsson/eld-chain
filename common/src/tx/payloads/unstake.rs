use crate::coin::Coin;
use crate::error::EldError;
use crate::tx::parts::{HasAmount, HasSender, TxAmount};
use crate::Address;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
#[serde(try_from = "UnstakeTxUnchecked")]
pub struct UnstakeTx {
    pub sender: Address,
    #[serde(with = "crate::tx::parts::amount::tx_amount_string")]
    pub amount: TxAmount,
}

/// Wire shape for [`UnstakeTx`]. With `try_from`, unknown-field rejection must be on this struct
/// (not only on [`UnstakeTx`]); otherwise untagged [`PayloadInner`] can match unstake for stake
/// JSON and drop `public_key`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnstakeTxUnchecked {
    sender: Address,
    #[serde(with = "crate::tx::parts::amount::tx_amount_string")]
    amount: TxAmount,
}

impl TryFrom<UnstakeTxUnchecked> for UnstakeTx {
    type Error = EldError;

    fn try_from(unchecked: UnstakeTxUnchecked) -> Result<Self, Self::Error> {
        UnstakeTx::new(unchecked.sender, unchecked.amount)
    }
}

impl UnstakeTx {
    pub fn new(sender: Address, amount: TxAmount) -> Result<Self, EldError> {
        if amount == 0 {
            return Err(EldError::ValidationError {
                field: "unstake amount".to_string(),
                value: amount.to_string(),
                details: "Unstake amount cannot be zero".to_string(),
            });
        }

        let amount_coin = Coin::new(amount.into())?;
        if amount_coin > Coin::max() {
            return Err(EldError::ValidationError {
                field: "unstake amount".to_string(),
                value: amount.to_string(),
                details: "Unstake amount exceeds maximum coin value".to_string(),
            });
        }

        Ok(Self { sender, amount })
    }
}

impl std::fmt::Display for UnstakeTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "UnstakeTx {{\n sender: {}\n amount: {}\n }}",
            self.sender, self.amount
        )
    }
}

impl HasSender for UnstakeTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for UnstakeTx {
    fn amount(&self) -> u128 {
        self.amount.as_u128()
    }
}

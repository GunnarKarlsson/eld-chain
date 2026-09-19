use crate::coin::Coin;
use crate::error::EldError;
use crate::tx::parts::{HasAmount, HasSender, TxAmount};
use crate::Address;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(try_from = "TransferTxUnchecked")]
pub struct TransferTx {
    pub sender: Address,
    pub recipient: Address,
    #[serde(with = "crate::tx::parts::amount::tx_amount_string")]
    pub amount: TxAmount,
}

/// Wire/decoding shape for [`TransferTx`]; validation runs in [`TransferTx::new`].
#[derive(Deserialize)]
struct TransferTxUnchecked {
    sender: Address,
    recipient: Address,
    #[serde(with = "crate::tx::parts::amount::tx_amount_string")]
    amount: TxAmount,
}

impl TryFrom<TransferTxUnchecked> for TransferTx {
    type Error = EldError;

    fn try_from(unchecked: TransferTxUnchecked) -> Result<Self, Self::Error> {
        TransferTx::new(unchecked.sender, unchecked.recipient, unchecked.amount)
    }
}

impl TransferTx {
    /// Builds a transfer after validating sender, recipient, and amount.
    pub fn new(sender: Address, recipient: Address, amount: TxAmount) -> Result<Self, EldError> {
        if sender == recipient {
            return Err(EldError::ValidationError {
                field: "transfer transaction".to_string(),
                value: format!("sender: {sender}, recipient: {recipient}"),
                details: "Sender and recipient cannot be the same".to_string(),
            });
        }

        let amount_coin = Coin::new(amount.into()).map_err(|_| EldError::ValidationError {
            field: "transfer amount".to_string(),
            value: amount.to_string(),
            details: "Invalid transfer amount (must be non-negative and within bounds)".to_string(),
        })?;

        let value: u128 = amount_coin.into();
        if value == 0 {
            EldError::validation_error(
                "transfer amount",
                &value.to_string(),
                "Transfer amount cannot be zero",
            )?;
        }

        Ok(Self {
            sender,
            recipient,
            amount,
        })
    }
}

impl std::fmt::Display for TransferTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TransferTx {{\n sender: {}\n recipient: {}\n amount: {}\n }}",
            self.sender, self.recipient, self.amount
        )
    }
}

impl HasSender for TransferTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for TransferTx {
    fn amount(&self) -> u128 {
        self.amount.as_u128()
    }
}

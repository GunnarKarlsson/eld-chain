use crate::coin::Coin;
use crate::error::EldError;
use crate::tx::parts::{HasAmount, HasSender, TxAmount};
use crate::Address;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(try_from = "StakeTxUnchecked")]
pub struct StakeTx {
    pub sender: Address,
    #[serde(with = "crate::tx::parts::amount::tx_amount_string")]
    pub amount: TxAmount,
    #[serde(default)]
    pub public_key: Option<String>, // Ed25519 public key in hex format
}

#[derive(Deserialize)]
struct StakeTxUnchecked {
    sender: Address,
    #[serde(with = "crate::tx::parts::amount::tx_amount_string")]
    amount: TxAmount,
    #[serde(default)]
    public_key: Option<String>,
}

impl TryFrom<StakeTxUnchecked> for StakeTx {
    type Error = EldError;

    fn try_from(unchecked: StakeTxUnchecked) -> Result<Self, Self::Error> {
        StakeTx::new(unchecked.sender, unchecked.amount, unchecked.public_key)
    }
}

fn stake_validate_public_key(pk: &str, expected_len: usize) -> Result<(), EldError> {
    if pk.len() != expected_len {
        return Err(EldError::ValidationError {
            field: "public key".to_string(),
            value: pk.to_string(),
            details: format!(
                "Public key must be {} hex characters, got {} characters",
                expected_len,
                pk.len()
            ),
        });
    }
    if !pk.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(EldError::ValidationError {
            field: "public key".to_string(),
            value: pk.to_string(),
            details: "Public key must be valid hex".to_string(),
        });
    }
    Ok(())
}

impl StakeTx {
    pub fn new(
        sender: Address,
        amount: TxAmount,
        public_key: Option<String>,
    ) -> Result<Self, EldError> {
        let amount_coin = Coin::new(amount.into())?;
        if amount_coin < Coin::new(crate::constants::protocol::MIN_STAKE_AMOUNT)? {
            EldError::validation_error(
                "stake amount",
                &amount.to_string(),
                &format!(
                    "Stake amount {} is below minimum required {}",
                    amount_coin,
                    Coin::new(crate::constants::protocol::MIN_STAKE_AMOUNT)?
                ),
            )?;
        }

        if let Some(ref pk) = public_key {
            stake_validate_public_key(pk, 64)?;
        }

        Ok(Self {
            sender,
            amount,
            public_key,
        })
    }
}

impl std::fmt::Display for StakeTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "StakeTx {{\n sender: {}\n amount: {}\n public_key: {:?}\n }}",
            self.sender, self.amount, self.public_key
        )
    }
}

impl HasSender for StakeTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for StakeTx {
    fn amount(&self) -> u128 {
        self.amount.as_u128()
    }
}

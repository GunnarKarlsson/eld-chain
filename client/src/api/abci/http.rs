use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use bincode;
use eld_common::account::Account;
use eld_common::address::Address;
use eld_common::cado::CadoType;
use eld_common::cado::{CadoBody, CadoPath, CadoPathKey};
use eld_common::error::EldError;
use eld_common::staking_account::StakingAccount;
use eld_common::validator::{ActiveValidatorsInfo, CapacityValidatorsInfo, EpochInfo};
use serde::{self, Deserialize, Serialize};
use serde_json;
use std::convert::From;
use std::fmt;
use std::str::FromStr;
use tendermint::block::Height;
use tendermint::AppHash;
use tendermint_rpc::endpoint::block::Response;
use tendermint_rpc::query::Query;
use tendermint_rpc::{Client, HttpClient, Order};

/// Placeholder for Tendermint ABCI Info `data` JSON. Currently unused (empty object).
#[derive(Debug, Serialize, Deserialize)]
pub struct AppInfoData {}

/// HTTP client for Tendermint RPC and Eld ABCI queries (`abci_query`, blocks, tx search).
pub struct AbciHttpApi {
    client: HttpClient,
}

impl AbciHttpApi {
    /// Connect to Tendermint RPC at `base_url` (for example `http://127.0.0.1:26657/`).
    pub fn new(base_url: String) -> Result<Self, EldError> {
        let client = HttpClient::new(base_url.as_str()).map_err(|e| EldError::NetworkError {
            operation: "create ABCI HTTP client".to_string(),
            details: format!("Failed to create HTTP client for '{base_url}': {e}"),
        })?;
        Ok(Self { client })
    }
}

impl AbciHttpApi {
    pub async fn get_active_validators(&self) -> Result<Option<ActiveValidatorsInfo>, EldError> {
        let response = self
            .client
            .abci_query(
                Some("active_validators".to_string()),
                Vec::new(),
                None,
                false,
            )
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "get_active_validators".to_string(),
                details: format!("Failed to query active validators: {e}"),
            })?;

        if response.info.is_empty() {
            return Ok(None);
        }

        let info: ActiveValidatorsInfo =
            serde_json::from_str(&response.info).map_err(|e| EldError::ValidationError {
                field: "active_validators_response".to_string(),
                value: response.info.clone(),
                details: format!("Failed to parse active validators response: {e}"),
            })?;
        Ok(Some(info))
    }

    pub async fn get_epoch_info(&self) -> Result<Option<EpochInfo>, EldError> {
        let response = self
            .client
            .abci_query(Some("epoch_info".to_string()), Vec::new(), None, false)
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "get_epoch_info".to_string(),
                details: format!("Failed to query epoch info: {e}"),
            })?;

        if response.info.is_empty() {
            return Ok(None);
        }

        let info: EpochInfo =
            serde_json::from_str(&response.info).map_err(|e| EldError::ValidationError {
                field: "epoch_info_response".to_string(),
                value: response.info.clone(),
                details: format!("Failed to parse epoch info response: {e}"),
            })?;
        Ok(Some(info))
    }

    /// Query registered capacity validators from ABCI (`capacity_validators` path).
    pub async fn get_capacity_validators(
        &self,
    ) -> Result<Option<CapacityValidatorsInfo>, EldError> {
        let response = self
            .client
            .abci_query(
                Some(eld_common::constants::abci_query::ACTIVE_CAPACITY_VALIDATORS.to_string()),
                Vec::new(), // data: empty string
                None,       // height: None (latest)
                false,      // prove: false
            )
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "get_capacity_validators".to_string(),
                details: format!("Failed to query capacity providers: {e}"),
            })?;

        if response.info.is_empty() {
            return Ok(None);
        }

        let info: CapacityValidatorsInfo =
            serde_json::from_str(&response.info).map_err(|e| EldError::ValidationError {
                field: "capacity_validators_response".to_string(),
                value: response.info.clone(),
                details: format!("Failed to parse capacity providers response: {e}"),
            })?;
        Ok(Some(info))
    }

    /// Check if a specific capacity validator is registered.
    pub async fn is_capacity_provider_registered(
        &self,
        provider_address: &str,
    ) -> Result<bool, EldError> {
        let providers_info = self.get_capacity_validators().await?;

        if let Some(info) = providers_info {
            let parsed = Address::parse_hex_str(provider_address)?;
            let is_registered = info
                .capacity_validators
                .iter()
                .any(|sp| sp.address == parsed);
            Ok(is_registered)
        } else {
            Ok(false)
        }
    }

    pub async fn get_account_by_address(&self, address: &str) -> Result<Option<Account>, EldError> {
        let addr = Address::parse_hex_str(address)?;
        let cado_path = CadoPath::new(CadoType::Account, CadoPathKey::Address(addr))?;

        // Query using CADO path
        let response = self
            .client
            .abci_query(
                Some("cado".to_string()),
                cado_path.as_str().as_bytes().to_vec(),
                None,
                false,
            )
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "get_account_by_address".to_string(),
                details: format!(
                    "Failed to query account for address {}: {e}",
                    addr.hex_with_prefix()
                ),
            })?;

        if response.info.is_empty() {
            return Ok(None);
        }

        // Parse CADO response
        let cado: CadoBody =
            serde_json::from_str(&response.info).map_err(|e| EldError::ValidationError {
                field: "account_cado_response".to_string(),
                value: response.info.clone(),
                details: format!("Failed to parse CADO response for account: {e}"),
            })?;

        // Extract account data from CADO
        let account_data = match cado {
            CadoBody::Mutable(cado_mut) => cado_mut.data().to_vec(),
            CadoBody::Immutable(cado) => cado.data().to_vec(),
        };

        // Deserialize account data
        let account: Account = Account::deserialize_bin(&account_data)?;
        Ok(Some(account))
    }

    pub async fn get_staking_account(
        &self,
        address: &str,
    ) -> Result<Option<StakingAccount>, EldError> {
        let addr = Address::parse_hex_str(address)?;
        let cado_path = CadoPath::new(CadoType::StakingAccount, CadoPathKey::Address(addr))?;

        // Query using CADO path
        let response = self
            .client
            .abci_query(
                Some("cado".to_string()),
                cado_path.as_str().as_bytes().to_vec(),
                None,
                false,
            )
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "get_staking_account".to_string(),
                details: format!(
                    "Failed to query staking account for address {}: {e}",
                    addr.hex_with_prefix()
                ),
            })?;

        if response.info.is_empty() {
            return Ok(None);
        }

        // Parse CADO response
        let cado: CadoBody =
            serde_json::from_str(&response.info).map_err(|e| EldError::ValidationError {
                field: "staking_account_cado_response".to_string(),
                value: response.info.clone(),
                details: format!("Failed to parse CADO response for staking account: {e}"),
            })?;

        // Extract staking account data from CADO
        let staking_account_data = match cado {
            CadoBody::Mutable(cado_mut) => cado_mut.data().to_vec(),
            CadoBody::Immutable(cado) => cado.data().to_vec(),
        };

        // Deserialize staking account data
        let staking_account: StakingAccount =
            bincode::deserialize(&staking_account_data).map_err(|e| EldError::ValidationError {
                field: "staking_account_data".to_string(),
                value: format!("{staking_account_data:?}"),
                details: format!("Failed to deserialize staking account data: {e}"),
            })?;
        Ok(Some(staking_account))
    }

    pub async fn get_latest_abci_info(&self) -> Result<AbciInfoWrapper, EldError> {
        let abci_info = self
            .client
            .abci_info()
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "get_latest_abci_info".to_string(),
                details: format!("Failed to get ABCI info: {e}"),
            })?;
        Ok(AbciInfoWrapper::from(abci_info))
    }
    pub async fn get_block(&self, height: u64) -> Result<Response, EldError> {
        let height = Height::try_from(height).map_err(|e| EldError::ValidationError {
            field: "block_height".to_string(),
            value: height.to_string(),
            details: format!("Failed to convert u64 to Height: {e}"),
        })?;
        let block = self
            .client
            .block(height)
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "get_block".to_string(),
                details: format!("Failed to get block at height {height}: {e}"),
            })?;
        Ok(block)
    }

    /// Fetch a committed transaction and its execution result by hash.
    pub async fn get_tx_by_hash(
        &self,
        hash: tendermint::Hash,
    ) -> Result<tendermint_rpc::endpoint::tx::Response, EldError> {
        self.client
            .tx(hash, false)
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "get_tx_by_hash".to_string(),
                details: format!("Failed to get transaction {hash}: {e}"),
            })
    }

    pub async fn get_transactions_for_account(
        &self,
        address: String,
    ) -> Result<Vec<tendermint_rpc::endpoint::tx::Response>, EldError> {
        // No 'OR' queries allowed by api so we do multiple queries
        let queries = [
            format!("Transfer.recipient='{address}'"),
            format!("Transfer.sender='{address}'"),
            format!("AddMetadata.sender='{address}'"),
            format!("Stake.sender='{address}'"),
            format!("Unstake.sender='{address}'"),
        ];
        let mut txs = Vec::new();
        for query in queries {
            txs.append(&mut self.query_node(&query).await?.txs);
        }
        Ok(txs)
    }

    pub async fn query_node(
        &self,
        query_str: &str,
    ) -> Result<tendermint_rpc::endpoint::tx_search::Response, EldError> {
        let q = Query::from_str(query_str).map_err(|e| EldError::ValidationError {
            field: "query_string".to_string(),
            value: query_str.to_string(),
            details: format!("Failed to parse query string: {e}"),
        })?;
        let prove = false; // don't include proofs
        let page = 1;
        let per_page = 30;
        self.client
            .tx_search(q, prove, page, per_page, Order::Ascending)
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "query_node".to_string(),
                details: format!("Query failed for '{query_str}': {e}"),
            })
    }

    pub async fn get_cado(&self, path: String) -> Result<serde_json::Value, EldError> {
        // query using abci
        let response = self
            .client
            .abci_query(
                Some("cado".to_string()),
                path.as_bytes().to_vec(),
                None,
                false,
            )
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "get_cado".to_string(),
                details: format!("Failed to query CADO at path {path}: {e}"),
            })?;
        if response.info.is_empty() {
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_str(&response.info).map_err(|e| EldError::ValidationError {
            field: "cado_response".to_string(),
            value: response.info.clone(),
            details: format!("Failed to parse CADO response: {e}"),
        })
    }

    pub async fn get_cados_by_prefix(&self, prefix: String) -> Result<Vec<CadoBody>, EldError> {
        let response = self
            .client
            .abci_query(
                Some("cado_list".to_string()),
                prefix.as_bytes().to_vec(),
                None,
                false,
            )
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "get_cados_by_prefix".to_string(),
                details: format!("Failed to query CADOs with prefix {prefix}: {e}"),
            })?;

        if response.info.is_empty() {
            return Ok(Vec::new());
        }

        serde_json::from_str(&response.info).map_err(|e| EldError::ValidationError {
            field: "cados_by_prefix_response".to_string(),
            value: response.info.clone(),
            details: format!("Failed to parse CADOs by prefix response: {e}"),
        })
    }

    pub async fn get_cado_paths(&self, search_string: String) -> Result<Vec<String>, EldError> {
        let response = self
            .client
            .abci_query(
                Some("cado_paths".to_string()),
                search_string.as_bytes().to_vec(),
                None,
                false,
            )
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "get_cado_paths".to_string(),
                details: format!(
                    "Failed to query CADO paths with search string {search_string}: {e}"
                ),
            })?;

        if response.info.is_empty() {
            return Ok(Vec::new());
        }

        serde_json::from_str(&response.info).map_err(|e| EldError::ValidationError {
            field: "cado_paths_response".to_string(),
            value: response.info.clone(),
            details: format!("Failed to parse CADO paths response: {e}"),
        })
    }

    /// Pinboard query (ABCI): request data is a UTF-8 path under [`eld_common::constants::cado::PATH_PREFIX_PINBOARD`].
    ///
    /// Returns parsed JSON from `response.info`.
    pub async fn pinboard_query(&self, path: String) -> Result<serde_json::Value, EldError> {
        let response = self
            .client
            .abci_query(
                Some(eld_common::constants::abci_query::PINBOARD.to_string()),
                path.as_bytes().to_vec(),
                None,
                false,
            )
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "pinboard_query".to_string(),
                details: format!("Failed to query pinboard path {path}: {e}"),
            })?;

        if response.info.is_empty() {
            return Ok(serde_json::Value::Null);
        }

        serde_json::from_str(&response.info).map_err(|e| EldError::ValidationError {
            field: "pinboard_query_response".to_string(),
            value: response.info.clone(),
            details: format!("Failed to parse pinboard query response: {e}"),
        })
    }
}

/// Tendermint transaction hash (SHA-256 of the wire bytes in the block).
pub fn wire_bytes_to_tx_hash(tx_bytes: &[u8]) -> tendermint::Hash {
    use sha2::{Digest, Sha256};
    let digest: [u8; 32] = Sha256::digest(tx_bytes).into();
    tendermint::Hash::Sha256(digest)
}

/// Decode an Eld transaction from hex-encoded JSON wire bytes (as in the block / mempool).
pub fn decode_eld_tx_from_wire(tx_bytes: &[u8]) -> Result<eld_common::tx::Tx, EldError> {
    let hex_str = std::str::from_utf8(tx_bytes).map_err(|e| EldError::ValidationError {
        field: "tx_bytes".to_string(),
        value: format!("{} bytes", tx_bytes.len()),
        details: format!("Transaction bytes are not UTF-8 hex: {e}"),
    })?;
    let json_bytes = hex::decode(hex_str).map_err(|e| EldError::ValidationError {
        field: "tx_hex".to_string(),
        value: hex_str.chars().take(64).collect(),
        details: format!("Failed to decode transaction hex: {e}"),
    })?;
    let json_str = String::from_utf8(json_bytes).map_err(|e| EldError::ValidationError {
        field: "tx_json".to_string(),
        value: "decoded hex".to_string(),
        details: format!("Decoded transaction is not UTF-8 JSON: {e}"),
    })?;
    serde_json::from_str(&json_str).map_err(|e| EldError::ValidationError {
        field: "tx".to_string(),
        value: json_str.chars().take(128).collect(),
        details: format!("Failed to parse Eld transaction JSON: {e}"),
    })
}

/// Decode one entry from `block.data.txs` (matches eld-chain-explorer `decodeEldTxFromBlockBase64`).
///
/// Tendermint JSON returns each tx as base64; the Rust RPC client yields the decoded bytes.
/// Primary path: UTF-8 hex string → hex → JSON. Fallbacks: direct JSON, nested base64.
pub fn decode_eld_tx_from_block_tx_bytes(tx_bytes: &[u8]) -> Result<eld_common::tx::Tx, EldError> {
    if let Ok(tx) = decode_eld_tx_from_wire(tx_bytes) {
        return Ok(tx);
    }

    if let Ok(json_str) = std::str::from_utf8(tx_bytes) {
        if let Ok(tx) = serde_json::from_str(json_str) {
            return Ok(tx);
        }
        if let Ok(inner) = BASE64_STANDARD.decode(json_str.trim()) {
            if let Ok(tx) = decode_eld_tx_from_block_tx_bytes(&inner) {
                return Ok(tx);
            }
        }
    }

    serde_json::from_slice(tx_bytes).map_err(|e| EldError::ValidationError {
        field: "tx".to_string(),
        value: format!("{} bytes", tx_bytes.len()),
        details: format!("Failed to decode Eld transaction from block tx bytes: {e}"),
    })
}

/// Whether a Tendermint `ExecTxResult` / deliver code indicates success (`0`).
pub fn tm_tx_result_is_success(code: tendermint::abci::Code) -> bool {
    code.value() == 0
}

/// Convert Tendermint ABCI events from `/tx` into `abci-rs` event types for indexing.
pub fn tm_events_to_abci_events(events: &[tendermint::abci::Event]) -> Vec<abci::types::Event> {
    use abci::types::{Event, EventAttribute};

    events
        .iter()
        .map(|e| Event {
            r#type: e.kind.clone(),
            attributes: e
                .attributes
                .iter()
                .map(|attr| EventAttribute {
                    key: attr.key_bytes().to_vec(),
                    value: attr.value_bytes().to_vec(),
                    index: attr.index(),
                })
                .collect(),
        })
        .collect()
}

// create custom type so we can implement Display (can't implement it for Info)
#[derive(Debug)]
pub struct AbciInfoWrapper {
    pub app_version: u64,
    pub version: String,
    pub last_block_height: Height,
    pub last_block_app_hash: AppHash,
}

impl From<tendermint::abci::response::Info> for AbciInfoWrapper {
    fn from(item: tendermint::abci::response::Info) -> Self {
        AbciInfoWrapper {
            app_version: item.app_version,
            version: item.version,
            last_block_height: item.last_block_height,
            last_block_app_hash: item.last_block_app_hash,
        }
    }
}

impl fmt::Display for AbciInfoWrapper {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "app_version: {}\nversion: {}\nlast_block_height: {}\nlast_block_app_hash: {}\n",
            self.app_version, self.version, self.last_block_height, self.last_block_app_hash,
        )?;
        writeln!(f, "Accounts:")
    }
}

pub struct QueryWrapper(pub tendermint_rpc::query::Query);

impl fmt::Display for QueryWrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_eld_tx_from_block_tx_bytes, decode_eld_tx_from_wire, AbciInfoWrapper};
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use base64::Engine;
    use ed25519_dalek::SigningKey;
    use eld_common::address::Address;
    use eld_common::tx::{Payload, TransferTx, Tx, TxAmount, TxPublicKey, TxSig};
    use tendermint::{block::Height, AppHash};

    fn sample_transfer_tx() -> Tx {
        let signing_key = SigningKey::from_bytes(&[1u8; 32]);
        let sender =
            Address::parse_hex_str("0x1234567890123456789012345678901234567890").expect("sender");
        let recipient = Address::parse_hex_str("0xfedcba0987654321fedcba0987654321fedcba09")
            .expect("recipient");
        Tx {
            sig: TxSig::new("0".repeat(128)).expect("64-byte zero signature hex"),
            nonce: 0u32.into(),
            fee: TxAmount::from(0u128),
            payload: Payload::new(
                TransferTx::new(sender, recipient, TxAmount::from(1u128)).expect("valid transfer"),
            ),
            public_key: TxPublicKey::from(signing_key.verifying_key()),
        }
    }

    #[test]
    fn decode_block_tx_bytes_hex_json_wire() {
        let tx = sample_transfer_tx();
        let json = serde_json::to_string(&tx).unwrap();
        let wire = hex::encode(json.as_bytes());
        let decoded = decode_eld_tx_from_block_tx_bytes(wire.as_bytes()).expect("decode");
        assert_eq!(decoded.payload.r#type, "Transfer");
    }

    #[test]
    fn decode_block_tx_bytes_base64_hex_json() {
        let tx = sample_transfer_tx();
        let json = serde_json::to_string(&tx).unwrap();
        let wire = hex::encode(json.as_bytes());
        let b64 = BASE64_STANDARD.encode(wire.as_bytes());
        let decoded = decode_eld_tx_from_block_tx_bytes(b64.as_bytes()).expect("decode b64");
        assert_eq!(decoded.payload.r#type, "Transfer");
        assert_eq!(
            decode_eld_tx_from_wire(wire.as_bytes()).unwrap().nonce,
            decoded.nonce
        );
    }

    #[test]
    fn convert_info_to_acbiinfowrapper() {
        let info = tendermint::abci::response::Info {
            app_version: 1,
            version: "2".to_owned(),
            last_block_height: Height::default(),
            last_block_app_hash: AppHash::default(),
            data: "{\"accounts\":{},\"metadata\":{}}".to_owned(),
        };
        let abci_info_wrapper = AbciInfoWrapper::from(info);
        assert!(!abci_info_wrapper.to_string().is_empty());
    }

    #[test]
    fn account_query_address_parse_accepts_optional_prefix() {
        // get_account_by_address / get_staking_account parse via Address::parse_hex_str
        // (no manual 0x prefix rewrite).
        let prefixed =
            Address::parse_hex_str("0x1234567890123456789012345678901234567890").expect("prefixed");
        let bare =
            Address::parse_hex_str("1234567890123456789012345678901234567890").expect("bare");
        assert_eq!(prefixed, bare);
        assert_eq!(
            prefixed.hex_with_prefix(),
            "0x1234567890123456789012345678901234567890"
        );
    }
}

//! Query an account balance and nonce via Tendermint RPC.
//!
//! Expects `config/config.json` in the current working directory (see `config/config.json.example`).
//!
//! ```sh
//! cargo run -p eld-client --example query_account -- 0xYourAddress
//! ```

use eld_client::config::ClientConfig;
use eld_client::ChainClient;
use eld_common::fee::FeeConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::args()
        .nth(1)
        .expect("usage: query_account <0x-address>");

    let config = ClientConfig::from_file("config/config.json")?;
    let client = ChainClient::new(config, FeeConfig::default());

    match client.get_account(address).await? {
        Some(account) => {
            println!("balance={}", account.balance().amount());
            println!("nonce={}", account.nonce().value());
        }
        None => println!("account not found"),
    }

    Ok(())
}

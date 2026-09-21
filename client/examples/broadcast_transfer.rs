//! Sign and broadcast a transfer from a named local wallet.
//!
//! Expects `config/config.json`, `config/consensus_config.json`, and `wallets/wallets.json`
//! in the current working directory.
//!
//! ```sh
//! cargo run -p eld-client --example broadcast_transfer -- my-wallet 0xRecipient 1000
//! ```

use eld_client::config::{get_client_setup, WALLETS_PATH};
use eld_client::ChainClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: broadcast_transfer <wallet_name> <recipient_0x> <amount>");
        std::process::exit(1);
    }

    let wallet_name = &args[1];
    let recipient = &args[2];
    let amount: u128 = args[3].parse()?;

    let setup = get_client_setup()?;
    let client = ChainClient::with_wallets(setup.config, setup.fee_config, WALLETS_PATH)?;

    let submitted = client
        .transfer(wallet_name.clone(), recipient.clone(), amount)
        .await?;

    println!("tx_hash={}", submitted.tx_hash);
    Ok(())
}

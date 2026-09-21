//! Log `broadcast_tx_commit` deliver_tx events (moved out of `eld-client`).

use eld_client::api::abci::deliver_tx_events;
use serde_json::Value;
use tracing::info;

pub fn log_deliver_tx_events(response: &Value) {
    for event in deliver_tx_events(response) {
        info!("\nEvent Type: {}", event.event_type);
        for (key, value) in event.attributes {
            info!("{key}: {value}");
        }
    }
}

use tokio::sync::broadcast;
use tracing::{error, info, warn};

use crate::errors::log_server_io_error_details;

/// Handle server startup errors with detailed logging and graceful failure
pub async fn handle_server_startup<T>(
    server_name: String,
    server_future: T,
    shutdown_tx: broadcast::Sender<()>,
) -> tokio::task::JoinHandle<()>
where
    T: std::future::Future<
            Output = std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>,
        > + Send
        + 'static,
{
    tokio::spawn(async move {
        let result = server_future.await;
        match result {
            Ok(_) => {
                info!("{} shut down gracefully", server_name);
            }
            Err(e) => {
                error!("{} failed: {}", server_name, e);
                log_server_io_error_details(&server_name, e.as_ref());
                let _ = shutdown_tx.send(());
            }
        }
    })
}

async fn os_shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "Failed to install SIGTERM handler");
                    let _ = ctrl_c.await;
                    return;
                }
            };
        tokio::select! {
            _ = ctrl_c => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}

/// Wait until a server task ends or the process receives Ctrl-C / SIGTERM, then abort remaining tasks.
pub async fn wait_and_shutdown(
    mut content_server_handle: tokio::task::JoinHandle<()>,
    mut consensus_server_handle: tokio::task::JoinHandle<()>,
    shutdown_tx: broadcast::Sender<()>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let mut content_completed = false;
    let mut consensus_completed = false;

    loop {
        tokio::select! {
            result = &mut content_server_handle, if !content_completed => {
                content_completed = true;
                match result {
                    Ok(_) => {
                        info!("Content server task completed");
                    }
                    Err(e) => {
                        error!("Content server task panicked: {}", e);
                    }
                }
            }
            result = &mut consensus_server_handle, if !consensus_completed => {
                consensus_completed = true;
                match result {
                    Ok(_) => {
                        info!("Consensus server task completed");
                    }
                    Err(e) => {
                        error!("Consensus server task panicked: {}", e);
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Shutdown signal received, initiating graceful shutdown");
                break;
            }
            _ = os_shutdown_signal() => {
                info!("OS shutdown signal received, initiating graceful shutdown");
                let _ = shutdown_tx.send(());
                break;
            }
        }

        if content_completed || consensus_completed {
            let _ = shutdown_tx.send(());
            break;
        }
    }

    let shutdown_timeout = tokio::time::Duration::from_secs(10);
    let shutdown_future = tokio::time::timeout(shutdown_timeout, async {
        content_server_handle.abort();
        consensus_server_handle.abort();
        let _ = tokio::join!(content_server_handle, consensus_server_handle);
    });

    match shutdown_future.await {
        Ok(_) => {
            info!("All servers shut down gracefully");
        }
        Err(_) => {
            warn!("Some servers did not shut down within timeout, forcing exit");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    #[tokio::test]
    async fn test_server_error_handling() {
        let (shutdown_tx, _shutdown_rx) = broadcast::channel::<()>(1);

        let failing_server = async {
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "Port already in use",
            )) as Box<dyn std::error::Error + Send + Sync>)
        };

        let handle =
            handle_server_startup("Test Server".to_string(), failing_server, shutdown_tx).await;

        let result = handle.await;
        assert!(result.is_ok());
    }
}

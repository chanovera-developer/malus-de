use malus_de_common::ipc::{IpcClient, IpcMessage};
use malus_de_common::logging::init_logging;
use tracing::info;

pub mod calculator;
pub mod indexer;

fn main() {
    init_logging("malus_de_spotlight");
    info!("Alternando Spotlight nativo del compositor vía IPC...");
    if let Err(error) = IpcClient::send_command(IpcMessage::ToggleSpotlight) {
        tracing::error!("No se pudo enviar ToggleSpotlight al compositor: {error}");
    }
}

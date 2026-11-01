use crate::accountant::clients::ClientMap;
use crate::{Accountant, Message};
use tokio::sync::mpsc;

/// Actor loop: applies every message received on `receiver` until the channel
/// closes, then returns the resulting clients.
pub async fn run(mut receiver: mpsc::Receiver<Message>) -> ClientMap {
    let mut accountant = Accountant::new();
    while let Some(message) = receiver.recv().await {
        accountant.apply(message);
    }
    accountant.into_clients()
}

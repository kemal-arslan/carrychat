//! Bluetooth LE (§9.5) on desktops, through btleplug. The node looks for
//! nodes that advertise the CarryChat service and connects to them; it does not
//! advertise itself, since btleplug can only connect. So a desktop reaches
//! phones over Bluetooth, and desktops reach each other over the local network.

use std::sync::{Arc, Weak};
use std::time::Duration;

use btleplug::api::{
    Central, CentralEvent, CentralState, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::{uuid, Uuid};

use crate::sync::{self, Link};
use crate::{Error, Result, Shared};

/// The CarryChat service, which a node advertises with its session id as the
/// service data.
pub const SERVICE: Uuid = uuid!("43524348-a0c5-4a3e-9b31-6f2d8e7c1a01");
/// The characteristic the stream crosses: the connecting side writes to it,
/// the advertising side indicates on it.
pub const DATA: Uuid = uuid!("43524348-a0c5-4a3e-9b31-6f2d8e7c1a02");

/// Bytes per write: the most an attribute value holds. Writes wait for their
/// response, as macOS drops writes without one when its queue is full, and
/// that would break the stream. Over a smaller MTU, macOS sends a write this
/// long as prepared writes, which the other side joins.
const WRITE_LEN: usize = 512;
const SWEEP_INTERVAL: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const BUFFER_LEN: usize = 64 * 1024;

pub(crate) async fn start(shared: &Arc<Shared>) -> Result<()> {
    let manager = Manager::new().await?;
    let adapter = manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .ok_or(Error::Invalid("there is no Bluetooth adapter"))?;
    if let Ok(state) = adapter.adapter_state().await {
        note_radio(shared, state);
    }
    tokio::spawn(watch_radio(Arc::downgrade(shared), adapter.clone()));
    adapter
        .start_scan(ScanFilter {
            services: vec![SERVICE],
        })
        .await?;
    tokio::spawn(sweep_loop(Arc::downgrade(shared), adapter));
    Ok(())
}

/// Notes whether Bluetooth is on whenever that changes, for the status.
async fn watch_radio(shared: Weak<Shared>, adapter: Adapter) {
    let Ok(mut events) = adapter.events().await else {
        return;
    };
    while let Some(event) = events.next().await {
        let CentralEvent::StateUpdate(state) = event else {
            continue;
        };
        let Some(shared) = shared.upgrade() else {
            return;
        };
        note_radio(&shared, state);
    }
}

fn note_radio(shared: &Shared, state: CentralState) {
    match state {
        CentralState::PoweredOn => shared.set_bluetooth_radio(true),
        CentralState::PoweredOff => shared.set_bluetooth_radio(false),
        CentralState::Unknown => {}
    }
}

/// Every few seconds, connects to the CarryChat nodes seen so far that are not
/// connected. Scanning reports a device once, so its events alone would not
/// bring back a node that went out of range and returned.
async fn sweep_loop(shared: Weak<Shared>, adapter: Adapter) {
    let mut tick = tokio::time::interval(SWEEP_INTERVAL);
    loop {
        tick.tick().await;
        let Some(shared) = shared.upgrade() else {
            return;
        };
        let Ok(peripherals) = adapter.peripherals().await else {
            continue;
        };
        for peripheral in peripherals {
            let Some(peer) = session_of(&peripheral).await else {
                continue;
            };
            if !shared.peers.should_dial(peer, true) {
                continue;
            }
            let shared = shared.clone();
            tokio::spawn(async move {
                if connect(shared.clone(), peripheral, peer).await.is_err() {
                    shared.peers.dial_ended(peer);
                }
            });
        }
    }
}

async fn session_of(peripheral: &Peripheral) -> Option<u64> {
    let properties = peripheral.properties().await.ok()??;
    let data = properties.service_data.get(&SERVICE)?;
    Some(u64::from_be_bytes(data.as_slice().try_into().ok()?))
}

/// Connects, then syncs until the connection ends.
async fn connect(shared: Arc<Shared>, peripheral: Peripheral, peer: u64) -> Result<()> {
    let ready = async {
        peripheral.connect().await?;
        peripheral.discover_services().await?;
        let data = peripheral
            .characteristics()
            .into_iter()
            .find(|characteristic| characteristic.uuid == DATA)
            .ok_or(Error::Invalid("the device has no CarryChat characteristic"))?;
        let notifications = peripheral.notifications().await?;
        peripheral.subscribe(&data).await?;
        Ok::<_, Error>((data, notifications))
    };
    let (data, mut notifications) = match tokio::time::timeout(CONNECT_TIMEOUT, ready).await {
        Ok(Ok(ready)) => ready,
        Ok(Err(error)) => {
            let _ = peripheral.disconnect().await;
            return Err(error);
        }
        Err(_) => {
            let _ = peripheral.disconnect().await;
            return Err(Error::Invalid("Bluetooth connection timed out"));
        }
    };

    let (sync_end, link_end) = tokio::io::duplex(BUFFER_LEN);
    let (sync_reader, sync_writer) = tokio::io::split(sync_end);
    let (mut link_reader, mut link_writer) = tokio::io::split(link_end);
    let incoming = tokio::spawn(async move {
        while let Some(notification) = notifications.next().await {
            if notification.uuid == DATA
                && link_writer.write_all(&notification.value).await.is_err()
            {
                break;
            }
        }
        // The device is gone: the sync reads the end of the stream.
        let _ = link_writer.shutdown().await;
    });
    let writer = peripheral.clone();
    let outgoing = tokio::spawn(async move {
        let mut buffer = vec![0; WRITE_LEN];
        while let Ok(len @ 1..) = link_reader.read(&mut buffer).await {
            if writer
                .write(&data, &buffer[..len], WriteType::WithResponse)
                .await
                .is_err()
            {
                break;
            }
        }
    });

    sync::run(
        shared,
        Link::Bluetooth,
        sync_reader,
        sync_writer,
        Some(peer),
    )
    .await;
    incoming.abort();
    outgoing.abort();
    let _ = peripheral.disconnect().await;
    Ok(())
}

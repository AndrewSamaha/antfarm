use anyhow::Result;
use mdns_sd::{Receiver, ServiceDaemon, ServiceEvent, ServiceInfo};
use std::{net::IpAddr, thread};
use tokio::sync::mpsc as tokio_mpsc;

const SERVICE_TYPE: &str = "_antfarm._tcp.local.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiscoverySource {
    Localhost,
    Mdns,
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredServer {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) addr: String,
    pub(crate) source: DiscoverySource,
}

#[derive(Debug, Clone)]
pub(crate) enum DiscoveryUpdate {
    Upsert(DiscoveredServer),
    Remove { id: String },
    Error(String),
}

pub(crate) fn localhost_server(port: u16) -> DiscoveredServer {
    DiscoveredServer {
        id: format!("localhost:{port}"),
        label: format!("Localhost ({port})"),
        addr: format!("127.0.0.1:{port}"),
        source: DiscoverySource::Localhost,
    }
}

pub(crate) fn spawn_mdns_discovery() -> tokio_mpsc::UnboundedReceiver<DiscoveryUpdate> {
    let (tx, rx) = tokio_mpsc::unbounded_channel();
    thread::spawn(move || {
        if let Err(error) = discovery_loop(&tx) {
            let _ = tx.send(DiscoveryUpdate::Error(error.to_string()));
        }
    });
    rx
}

fn discovery_loop(tx: &tokio_mpsc::UnboundedSender<DiscoveryUpdate>) -> Result<()> {
    let daemon = ServiceDaemon::new()?;
    let receiver = daemon.browse(SERVICE_TYPE)?;
    forward_events(tx, receiver);
    Ok(())
}

fn forward_events(
    tx: &tokio_mpsc::UnboundedSender<DiscoveryUpdate>,
    receiver: Receiver<ServiceEvent>,
) {
    while let Ok(event) = receiver.recv() {
        match event {
            ServiceEvent::ServiceResolved(info) => {
                if let Some(server) = resolved_server(info) {
                    let _ = tx.send(DiscoveryUpdate::Upsert(server));
                }
            }
            ServiceEvent::ServiceRemoved(_, fullname) => {
                let _ = tx.send(DiscoveryUpdate::Remove { id: fullname });
            }
            _ => {}
        }
    }
}

fn resolved_server(info: ServiceInfo) -> Option<DiscoveredServer> {
    let addr = info
        .get_addresses()
        .iter()
        .find(|addr| matches!(addr, IpAddr::V4(_)))
        .or_else(|| info.get_addresses().iter().next())?;
    let port = info.get_port();
    let fullname = info.get_fullname().to_string();
    Some(DiscoveredServer {
        id: fullname.clone(),
        label: format!("{} ({})", info.get_hostname(), fullname),
        addr: format!("{addr}:{port}"),
        source: DiscoverySource::Mdns,
    })
}

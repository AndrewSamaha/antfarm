use anyhow::Result;
use if_addrs::get_if_addrs;
use mdns_sd::{Receiver, ServiceDaemon, ServiceEvent, ServiceInfo};
use std::{collections::HashSet, net::IpAddr, thread};
use tokio::{net::TcpStream, time::{Duration, timeout}};
use tokio::sync::mpsc as tokio_mpsc;

const SERVICE_TYPE: &str = "_antfarm._tcp.local.";
const LOCALHOST_PROBE_TIMEOUT: Duration = Duration::from_millis(150);

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
    pub(crate) ip: IpAddr,
    pub(crate) port: u16,
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
        ip: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        port,
        source: DiscoverySource::Localhost,
    }
}

pub(crate) async fn probe_localhost_server(port: u16) -> Option<DiscoveredServer> {
    let addr = format!("127.0.0.1:{port}");
    match timeout(LOCALHOST_PROBE_TIMEOUT, TcpStream::connect(&addr)).await {
        Ok(Ok(_stream)) => Some(localhost_server(port)),
        _ => None,
    }
}

pub(crate) fn spawn_mdns_discovery(
    localhost_port: Option<u16>,
) -> tokio_mpsc::UnboundedReceiver<DiscoveryUpdate> {
    let (tx, rx) = tokio_mpsc::unbounded_channel();
    thread::spawn(move || {
        if let Err(error) = discovery_loop(&tx, localhost_port) {
            let _ = tx.send(DiscoveryUpdate::Error(error.to_string()));
        }
    });
    rx
}

fn discovery_loop(
    tx: &tokio_mpsc::UnboundedSender<DiscoveryUpdate>,
    localhost_port: Option<u16>,
) -> Result<()> {
    let daemon = ServiceDaemon::new()?;
    let receiver = daemon.browse(SERVICE_TYPE)?;
    let local_ips = if localhost_port.is_some() {
        Some(local_machine_ips()?)
    } else {
        None
    };
    forward_events(tx, receiver, localhost_port, local_ips.as_ref());
    Ok(())
}

fn forward_events(
    tx: &tokio_mpsc::UnboundedSender<DiscoveryUpdate>,
    receiver: Receiver<ServiceEvent>,
    localhost_port: Option<u16>,
    local_ips: Option<&HashSet<IpAddr>>,
) {
    while let Ok(event) = receiver.recv() {
        match event {
            ServiceEvent::ServiceResolved(info) => {
                if let Some(server) = resolved_server(info)
                    && !should_suppress_mdns_server(&server, localhost_port, local_ips)
                {
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
    let ip = info
        .get_addresses()
        .iter()
        .find(|addr| matches!(addr, IpAddr::V4(_)))
        .or_else(|| info.get_addresses().iter().next())?;
    let port = info.get_port();
    let fullname = info.get_fullname().to_string();
    Some(DiscoveredServer {
        id: fullname.clone(),
        label: format!("{} ({})", info.get_hostname(), fullname),
        addr: format!("{ip}:{port}"),
        ip: *ip,
        port,
        source: DiscoverySource::Mdns,
    })
}

fn local_machine_ips() -> Result<HashSet<IpAddr>> {
    let mut ips = HashSet::new();
    ips.insert(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    ips.insert(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST));
    for iface in get_if_addrs()? {
        ips.insert(iface.ip());
    }
    Ok(ips)
}

fn should_suppress_mdns_server(
    server: &DiscoveredServer,
    localhost_port: Option<u16>,
    local_ips: Option<&HashSet<IpAddr>>,
) -> bool {
    server.source == DiscoverySource::Mdns
        && localhost_port.is_some_and(|port| port == server.port)
        && local_ips.is_some_and(|ips| ips.contains(&server.ip))
}

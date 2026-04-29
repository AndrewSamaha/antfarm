use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceInfo};

const SERVICE_TYPE: &str = "_antfarm._tcp.local.";

pub(crate) struct MdnsRegistration {
    _daemon: ServiceDaemon,
    fullname: String,
}

impl MdnsRegistration {
    pub(crate) fn fullname(&self) -> &str {
        &self.fullname
    }
}

pub(crate) fn start_mdns_registration(port: u16) -> Result<MdnsRegistration> {
    let daemon = ServiceDaemon::new().context("create mDNS daemon")?;
    let instance_name = format!("antfarm-{port}");
    let host_name = format!("{instance_name}.local.");
    let service = ServiceInfo::new(
        SERVICE_TYPE,
        &instance_name,
        &host_name,
        "127.0.0.1",
        port,
        &[] as &[(&str, &str)],
    )
    .context("build mDNS service info")?
    .enable_addr_auto();
    let fullname = service.get_fullname().to_string();
    daemon
        .register(service)
        .context("register mDNS service")?;
    Ok(MdnsRegistration {
        _daemon: daemon,
        fullname,
    })
}

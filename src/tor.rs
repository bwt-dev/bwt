use std::net;

use libtor::{HiddenServiceVersion, Tor, TorAddress, TorFlag};

use crate::error::OptionExt;
use crate::{Config, Result};

pub fn start_onion(config: &Config) -> Result<()> {
    let mut tor = Tor::new();

    let datadir = match config.tor_dir() {
        Some(x) => x,
        None => {
            warn!("Cannot determine tor data dir location, provide a path via --tor-dir");
            return Ok(());
        }
    };
    let hsdir = datadir.join("hs-dir");

    tor.flag(TorFlag::DataDirectory(datadir.to_str().req()?.into()))
        .flag(TorFlag::SocksPort(0))
        .flag(TorFlag::HiddenServiceDir(hsdir.to_str().req()?.into()))
        .flag(TorFlag::HiddenServiceVersion(HiddenServiceVersion::V3));

    #[cfg(feature = "electrum")]
    tor.flag(TorFlag::HiddenServicePort(
        TorAddress::Port(50001),
        Some(to_tor_addr(&config.electrum_rpc_addr())).into(),
    ));

    #[cfg(feature = "http")]
    tor.flag(TorFlag::HiddenServicePort(
        TorAddress::Port(80),
        Some(to_tor_addr(&config.http_server_addr)).into(),
    ));

    let handle = tor.start_background();

    // https://docs.rs/notify/4.0.15/notify/

    Ok(())
}

fn to_tor_addr(addr: &net::SocketAddr) -> TorAddress {
    TorAddress::AddressPort(addr.ip().to_string(), addr.port())
}

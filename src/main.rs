use bitcoincore_rpc::{Auth as RpcAuth, Client as RpcClient};

use rust_eps::addrman::AddrManager;
use rust_eps::error::Result;

fn main() -> Result<()> {
    stderrlog::new().verbosity(3).init()?;

    let rpc_url = "http://localhost:18888/".into();
    let rpc_auth = RpcAuth::UserPass("user3".into(), "password3".into());
    let client = RpcClient::new(rpc_url, rpc_auth)?;

    let manager = AddrManager::new(client);

    manager.update()?;

    Ok(())
}

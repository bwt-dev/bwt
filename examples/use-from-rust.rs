use bwt::types::RescanSince;
use bwt::{App, Config, Result};

fn main() -> Result<()> {
    let my_desc = "wpkh(tpubD6NzVbkrYhZ4Ya1aR2od7JTGK6b44cwKhWzrvrTeTWFrzGokdAGHrZLK6BdYwpx9K7EoY38LzHva3SWwF8yRrXM9x9DQ3jCGKZKt1nQEz7n/0/*)";

    // Initialize the config
    let config = Config {
        network: bitcoin::Network::Regtest,
        bitcoind_dir: Some("/home/satoshi/.bitcoin".into()),
        bitcoind_wallet: Some("bwt".into()),
        electrum_addr: Some("127.0.0.1:0".parse().unwrap()),
        descriptors: vec![(my_desc.parse().unwrap(), RescanSince::Timestamp(0))],
        verbose: 2,
        ..Default::default()
    };
    config.setup_logger(); // optional

    // Boot up bwt. The thread will be blocked until the initial sync is completed
    let app = App::boot(config, None)?;

    // The index is now ready for querying
    let query = app.query();
    log::info!("synced up to {:?}", query.get_tip()?);
    log::info!("utxos: {:?}", query.list_unspent(None, 0, None)?);
    log::info!("electrum running on {}", app.electrum_addr().unwrap());

    // Start syncing new blocks/transactions in the background
    let shutdown_tx = app.sync_background();

    // To shutdown the syncing thread, send a message to `shutdown_tx` or let it drop out of scope
    shutdown_tx.send(()).unwrap();

    Ok(())
}

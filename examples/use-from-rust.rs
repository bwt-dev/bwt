use bwt::{App, Config, Result};
use structopt::StructOpt;

// WARNING: The Rust API is still being evolved and is likely to break compatibility.
// Use the HTTP API if you want stronger backwards compatibility guarantees.

fn main() -> Result<()> {
    // Initialize the config
    let config = Config::from_args();
    config.setup_logger(); // optional

    // Boot up bwt. Blocks the thread until the initial sync is completed.
    let app = App::boot(config)?;

    // The index is now ready for querying
    let query = app.query();
    println!("{:?}", query.get_tip()?);

    // Start syncing new blocks/transactions in the background
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || app.sync(Some(shutdown_rx)));

    // You can shutdown the app by sending a message to `shutdown_tx`.
    // This will also happen automatically when its dropped out of scope.
    shutdown_tx.send(());

    Ok(())
}

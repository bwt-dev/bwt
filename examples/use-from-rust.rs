use bwt::{App, Config, Result};
use structopt::StructOpt;

// WARNING: The Rust API is still being evolved and is likely to break compatibility.
// Use the HTTP API if you want stronger backwards compatibility guarantees.

fn main() -> Result<()> {
    let config = Config::from_args(); // or construct manually with Config { ... }
    config.setup_logger(); // optional

    let app = App::boot(config)?;
    let query = app.query();

    std::thread::spawn(move || app.sync());

    println!("{:?}", query.get_tip()?);

    Ok(())
}

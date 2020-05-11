use log::Level;
use structopt::StructOpt;

use pxt::{App, Config, Result};

#[allow(unreachable_code)]
fn main() -> Result<()> {
    let config = Config::from_args();

    setup_logger(config.verbose);

    let app = App::boot(config)?;
    app.sync();

    Ok(())
}

fn setup_logger(verbose: usize) {
    pretty_env_logger::formatted_builder()
        .filter_module(
            "pxt",
            match verbose {
                0 => Level::Info,
                1 => Level::Debug,
                _ => Level::Trace,
            }
            .to_level_filter(),
        )
        .filter_module(
            "bitcoincore_rpc",
            match verbose {
                0 | 1 | 2 => Level::Warn,
                _ => Level::Debug,
            }
            .to_level_filter(),
        )
        .filter_module(
            "warp",
            match verbose {
                0 | 1 => Level::Info,
                2 => Level::Debug,
                _ => Level::Trace,
            }
            .to_level_filter(),
        )
        .filter_module(
            "hyper",
            Level::Warn.to_level_filter(),
        )
        .filter_level(
            match verbose {
                0 | 1 => Level::Warn,
                2 | 3 => Level::Info,
                4 => Level::Debug,
                _ => Level::Trace,
            }
            .to_level_filter(),
        )
        .init();
}

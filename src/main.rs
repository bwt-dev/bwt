use bwt::{App, Config, Result};
use structopt::StructOpt;

fn main() -> Result<()> {
    Config::dotenv();
    let config = Config::from_args();

    config.setup_logger();

    let app = App::boot(config, None)?;
    app.sync_loop(None);

    Ok(())
}

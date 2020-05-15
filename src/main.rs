use bwt::{App, Config, Result};
use structopt::StructOpt;

#[allow(unreachable_code)]
fn main() -> Result<()> {
    Config::dotenv();
    let config = Config::from_args();

    config.setup_logger();

    let app = App::boot(config)?;
    app.sync();

    Ok(())
}

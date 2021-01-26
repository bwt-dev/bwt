use bwt::{App, Config, Result};

fn main() -> Result<()> {
    Config::dotenv();
    let config = Config::from_args_env()?;

    config.setup_logger();

    let app = App::boot(config, None)?;
    app.sync_loop(None);

    Ok(())
}

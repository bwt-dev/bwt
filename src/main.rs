use pxt::{App, Config, Result};

#[allow(unreachable_code)]
fn main() -> Result<()> {
    let config = Config::from_args();

    stderrlog::new()
        .module(module_path!())
        .verbosity(2 + config.verbose)
        .init()?;

    let app = App::boot(config)?;
    app.sync();

    Ok(())
}

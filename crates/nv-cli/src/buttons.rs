use clap::Args;
use std::time::{Duration, Instant};

#[derive(Args)]
pub struct ButtonsArgs {
    /// Seconds to wait for presses.
    #[arg(long, default_value_t = 30)]
    seconds: u64,
}

/// Prints the presses of the buttons on the first film scanner, as the scanner portal reports them.
/// Through the portal only; libsane has no event for buttons.
pub fn run(args: ButtonsArgs) -> anyhow::Result<()> {
    let session = nv_acquire::make_backend().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut devices = session.get_devices(true).map_err(|e| anyhow::anyhow!("get_devices: {e}"))?;
    if devices.is_empty() {
        devices = session.select_devices("").map_err(|e| anyhow::anyhow!("select_devices: {e}"))?;
    }
    let dev = devices.first().ok_or_else(|| anyhow::anyhow!("no scanner"))?;
    let handle = session.open(&dev.name).map_err(|e| anyhow::anyhow!("open: {e}"))?;
    println!("{} {}: press a button on the scanner, for {} s", dev.vendor, dev.model, args.seconds);
    let end = Instant::now() + Duration::from_secs(args.seconds);
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        if let Some(press) = handle.next_button(left.min(Duration::from_secs(1))) {
            println!("button {} (the scanner's {})", press.button, press.option);
        }
    }
    Ok(())
}

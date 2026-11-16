pub fn run() -> anyhow::Result<()> {
    let dir = nv_calibrate::profiles_dir();
    let names = nv_calibrate::list_profiles();
    if names.is_empty() {
        println!("No scanner profiles in {}; build one with `nv-cli calibrate`.", dir.display());
    } else {
        println!("Scanner profiles in {}:", dir.display());
        for n in names {
            println!("  {n}");
        }
    }
    Ok(())
}

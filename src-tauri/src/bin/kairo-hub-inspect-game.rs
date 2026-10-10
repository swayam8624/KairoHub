//! Headless game-intake diagnostic used in CI and project support.
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("Kairo game inspection failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let source = args.next().map(PathBuf::from)
        .ok_or("usage: kairo-hub-inspect-game <original-game-root>")?;
    if args.next().is_some() {
        return Err("Only one source-game root may be inspected".into());
    }
    let report = kairo_hub_lib::source_game::inspect_source_game(&source)?;
    println!("{}", serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?);
    Ok(())
}

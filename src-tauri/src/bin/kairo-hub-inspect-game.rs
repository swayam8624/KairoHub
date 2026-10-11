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
    let source = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: kairo-hub-inspect-game <original-game-root> [--graph | --migration-plan]")?;
    let mode = match args.next() {
        None => "inspect",
        Some(flag) if flag == "--graph" => "graph",
        Some(flag) if flag == "--migration-plan" => "migration",
        Some(_) => return Err("Use optional --graph or --migration-plan".into()),
    };
    if args.next().is_some() {
        return Err("Too many CLI arguments".into());
    }
    if mode == "migration" {
        let report = kairo_hub_lib::source_game_migration::plan_godot_migration(&source)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else if mode == "graph" {
        let report = kairo_hub_lib::source_game_graph::inspect_godot_graph(&source)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        let report = kairo_hub_lib::source_game::inspect_source_game(&source)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    }
    Ok(())
}

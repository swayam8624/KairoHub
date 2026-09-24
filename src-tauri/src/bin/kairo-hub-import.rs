use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let root = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "usage: kairo-hub-import <root> <display-name> <engine-version> [entry-scene]".to_string())?;
    let display_name = arguments
        .next()
        .ok_or_else(|| "missing display name".to_string())?;
    let engine_version = arguments
        .next()
        .ok_or_else(|| "missing engine version".to_string())?;
    let entry_scene = arguments.next();
    if arguments.next().is_some() {
        return Err("too many arguments".into());
    }

    let project = kairo_hub_lib::project::import_external_gltf_directory(
        &root,
        entry_scene.as_deref(),
        &display_name,
        &engine_version,
    )?;
    println!("{}", project.display());
    Ok(())
}

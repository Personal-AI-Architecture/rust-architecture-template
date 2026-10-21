use std::process::Command;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let source = option(&args, "--source-checkout");
    let shared = option(&args, "--shared-validator");
    let mut commands: Vec<Vec<String>> = vec![
        vec!["run".into(), "--bin".into(), "paa-check".into()],
        vec!["fmt".into(), "--all".into(), "--".into(), "--check".into()],
        vec![
            "clippy".into(),
            "--all-targets".into(),
            "--all-features".into(),
            "--".into(),
            "-D".into(),
            "warnings".into(),
        ],
        vec!["test".into()],
        vec!["run".into(), "--bin".into(), "paa-acceptance".into()],
    ];
    if let Some(source) = &source {
        commands[0].extend(["--".into(), "--source-checkout".into(), source.clone()]);
    }
    for command in commands {
        if !run("cargo", &command) {
            std::process::exit(1);
        }
    }
    if let Some(shared) = shared {
        let mut validator = vec![shared, "--target".into(), "paa.target.json".into()];
        if let Some(source) = source {
            validator.extend(["--source-checkout".into(), source]);
        }
        if !run("node", &validator) {
            std::process::exit(1);
        }
    }
    println!("Rust PAA template validation passed.");
}
fn option(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}
fn run(program: &str, args: &[String]) -> bool {
    println!("\n[{program} {}]", args.join(" "));
    Command::new(program)
        .args(args)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

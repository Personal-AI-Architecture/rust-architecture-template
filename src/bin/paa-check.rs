use paa_rust_architecture_template::checks;
use std::path::PathBuf;

fn main() {
    let mut source = None;
    let args: Vec<_> = std::env::args().collect();
    if let Some(index) = args.iter().position(|arg| arg == "--source-checkout") {
        source = args.get(index + 1).map(PathBuf::from);
    }
    let root = std::env::current_dir().expect("working directory");
    let errors = checks::run(&root, source.as_deref());
    if errors.is_empty() {
        println!("Rust PAA checks passed.");
    } else {
        eprintln!("Rust PAA checks failed:");
        for error in errors {
            eprintln!("- {error}");
        }
        std::process::exit(1);
    }
}

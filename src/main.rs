fn main() {
    if let Err(error) = zpres::run_cli() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

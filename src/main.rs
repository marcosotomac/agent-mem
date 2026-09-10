use std::process;

fn main() {
    if let Err(e) = agent_mem::cli::run() {
        eprintln!("{}", e);
        process::exit(1);
    }
}


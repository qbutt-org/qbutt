use clap::Parser;

fn main() {
    let cli = framegen_manager_lib::Cli::parse();
    if cli.command.is_none() {
        eprintln!("No command provided. Use framegen-manager.exe --help.");
        std::process::exit(2);
    }
    framegen_manager_lib::run(cli)
}

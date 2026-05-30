use std::env;

use wecode::parse_cli_args;

mod app;

fn main() {
    let exit_code = match parse_cli_args(env::args()) {
        Ok(command) => match app::run(command) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("error: {error}");
                1
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            app::print_help();
            2
        }
    };

    std::process::exit(exit_code);
}

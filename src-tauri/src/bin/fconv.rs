//! `fconv` — console entry point for the File Converter CLI.

fn main() {
    std::process::exit(file_converter_lib::cli::run());
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tandem::diagnostics::install();
    let result = tandem::cli::run();
    if let Err(error) = &result {
        tandem::diagnostics::record_error("process exited with an error", error.as_ref());
    }
    result
}

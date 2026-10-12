//! Read-only smoke check. Prints only window labels, percentages and durations;
//! never opens Providock's database or prints account identity/authentication.
fn main() {
    match providock_lib::services::codex_connection::read_limits() {
        Ok(limits) => {
            println!("status=ok windows={}", limits.windows.len());
            for window in limits.windows {
                println!(
                    "window={} remaining_ratio={} duration_seconds={}",
                    window.label.as_deref().unwrap_or("unknown"),
                    window.remaining_ratio.as_deref().unwrap_or("unknown"),
                    window
                        .duration_seconds
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "unknown".into())
                );
            }
        }
        Err(error) => {
            // Error bodies can contain server diagnostics. Keep smoke output sanitized.
            eprintln!("status={:?}", error.status());
            std::process::exit(1);
        }
    }
}

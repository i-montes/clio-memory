//! What leaves this machine: nothing, but the downloads the person asks for. ONNX Runtime ships
//! Microsoft telemetry that it turns on by itself; the variable below turns it off.
/// The variable ONNX Runtime reads to keep its telemetry off.
pub const ORT_TELEMETRY_VAR: &str = "ORT_DISABLE_TELEMETRY";

/// Telemetry is off only when the variable is exactly `1`: any other value is not the runtime's.
pub fn telemetry_is_off(value: Option<&std::ffi::OsStr>) -> bool {
    value == Some(std::ffi::OsStr::new("1"))
}

/// Runs this program again with ONNX Runtime's telemetry off, unless it already is. Returns only
/// when nothing had to change (`Ok`) or the program could not be run again (`Err`): the caller
/// goes on either way, and Granite refuses to start the runtime without the variable.
pub fn private_runtime() -> Result<(), String> {
    if telemetry_is_off(std::env::var_os(ORT_TELEMETRY_VAR).as_deref()) {
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Err(run_again(&exe))
}

/// Replaces this process by `exe` with the same arguments and the variable set. Comes back, with
/// the reason, only if it could not.
fn run_again(exe: &std::path::Path) -> String {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(ORT_TELEMETRY_VAR, "1")
        .exec()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_value_1_turns_telemetry_off() {
        use std::ffi::OsStr;
        assert!(telemetry_is_off(Some(OsStr::new("1"))));
        for v in ["", "0", "true", "yes", " 1"] {
            assert!(!telemetry_is_off(Some(OsStr::new(v))), "{v:?}");
        }
        assert!(!telemetry_is_off(None));
    }

    /// A binary replaced by an update while it ran: the program goes on, it does not loop or die.
    #[test]
    fn a_program_that_cannot_be_run_again_says_why_and_returns() {
        let why = run_again(std::path::Path::new("/no/existe/clio-memory"));
        assert!(!why.is_empty());
    }
}

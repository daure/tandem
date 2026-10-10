use std::path::PathBuf;

pub(super) fn installed() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        // Persistent launch paths must survive replacement of the running executable inode.
        if let Some(path) = executable
            .as_os_str()
            .as_bytes()
            .strip_suffix(b" (deleted)")
        {
            return Ok(std::ffi::OsString::from_vec(path.to_vec()).into());
        }
    }
    Ok(executable)
}

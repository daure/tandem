use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use crate::store::environments::validate_name;

#[derive(Clone)]
pub(crate) struct Config {
    pub home: PathBuf,
    pub templates: PathBuf,
    pub workspaces: PathBuf,
    pub instructions: PathBuf,
    pub namespace: String,
    pub port: u16,
    pub keys: [char; 5],
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let home = match env::var_os("TANDEM_HOME") {
            Some(path) => PathBuf::from(path),
            None => dirs::data_local_dir()
                .ok_or("cannot determine data directory")?
                .join("tandem"),
        };
        let namespace = env::var("TANDEM_NAMESPACE").unwrap_or_else(|_| "tandem".into());
        let port = env::var("TANDEM_GATEWAY_PORT")
            .unwrap_or_else(|_| "9876".into())
            .parse::<u16>()
            .map_err(|_| "TANDEM_GATEWAY_PORT must be a port number")?;
        let mut config = Self::at(home, namespace, port)?;
        if let Some(path) = env::var_os("TANDEM_INSTRUCTIONS_FILE") {
            config.instructions =
                fs::canonicalize(path).map_err(|error| format!("instructions file: {error}"))?;
        }
        for (index, name) in ["INFO", "START", "NEW_TEMPLATE", "STOP", "REFRESH"]
            .iter()
            .enumerate()
        {
            if let Ok(value) = env::var(format!("TANDEM_KEY_{name}")) {
                if value.len() != 1 || !value.as_bytes()[0].is_ascii_lowercase() {
                    return Err(format!("TANDEM_KEY_{name} must be one lowercase letter"));
                }
                config.keys[index] = value.as_bytes()[0] as char;
            }
        }
        let unique: std::collections::BTreeSet<_> = config.keys.iter().collect();
        if unique.len() != config.keys.len() {
            return Err("Tandem action keys must be distinct".into());
        }
        Ok(config)
    }

    pub fn at(home: PathBuf, namespace: String, port: u16) -> Result<Self, String> {
        validate_name(&namespace)?;
        if !home.is_absolute() || port == 0 {
            return Err("TANDEM_HOME must be absolute and gateway port must be nonzero".into());
        }
        fs::create_dir_all(&home).map_err(|error| error.to_string())?;
        let home = fs::canonicalize(home).map_err(|error| error.to_string())?;
        let config = Self {
            templates: home.join("templates"),
            workspaces: home.join("workspaces"),
            instructions: home.join("instructions.md"),
            home,
            namespace,
            port,
            keys: ['i', 'n', 't', 's', 'r'],
        };
        for directory in [
            &config.templates,
            &config.workspaces,
            &config.home.join("locks"),
            &config.home.join("gateway"),
        ] {
            fs::create_dir_all(directory).map_err(|error| error.to_string())?;
            if !fs::canonicalize(directory)
                .map_err(|error| error.to_string())?
                .starts_with(&config.home)
            {
                return Err(format!(
                    "directory escapes TANDEM_HOME: {}",
                    directory.display()
                ));
            }
        }
        match private_file(&config.instructions, true) {
            Ok(mut file) => file
                .write_all(include_bytes!("../../agent-instructions.md"))
                .map_err(|error| error.to_string())?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
        Ok(config)
    }

    pub fn origin(&self) -> String {
        format!("http://localhost:{}", self.port)
    }
    pub fn project(&self, name: &str) -> String {
        format!("{}-{name}", self.namespace)
    }
    pub fn network(&self) -> String {
        self.project("gateway")
    }
}

pub(crate) fn private_file(path: &Path, create_new: bool) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true);
    if create_new {
        options.create_new(true);
    } else {
        options.create(true).truncate(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

pub(crate) fn read_text(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut text = String::new();
    file.take(262_145)
        .read_to_string(&mut text)
        .map_err(|error| error.to_string())?;
    if text.len() > 262_144 {
        return Err(format!("{} exceeds 256 KiB", path.display()));
    }
    Ok(text)
}

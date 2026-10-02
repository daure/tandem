use std::{fs, path::Path};

pub(super) fn read_cpu_temperature(root: &Path) -> Option<i32> {
    let mut temperatures = Vec::new();
    for device in fs::read_dir(root).ok()?.flatten() {
        let path = device.path();
        let Ok(name) = fs::read_to_string(path.join("name")) else {
            continue;
        };
        if name.trim() != "coretemp" {
            continue;
        }
        let Ok(sensors) = fs::read_dir(&path) else {
            continue;
        };
        for sensor in sensors.flatten() {
            let filename = sensor.file_name();
            let Some(stem) = filename
                .to_str()
                .and_then(|name| name.strip_suffix("_label"))
            else {
                continue;
            };
            if !stem.starts_with("temp") {
                continue;
            }
            let Ok(label) = fs::read_to_string(sensor.path()) else {
                continue;
            };
            if !label.trim().starts_with("Package id ") {
                continue;
            }
            if let Some(temperature) = fs::read_to_string(path.join(format!("{stem}_input")))
                .ok()
                .and_then(|value| value.trim().parse::<i32>().ok())
                .filter(|value| (0..=200_000).contains(value))
            {
                temperatures.push(temperature);
            }
        }
    }
    temperatures.into_iter().max()
}

#[cfg(test)]
#[path = "tests/cpu_temperature.rs"]
mod tests;

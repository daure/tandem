use super::*;

fn sensor(root: &Path, directory: &str, name: &str, label: &str, value: &str) {
    let path = root.join(directory);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("name"), name).unwrap();
    fs::write(path.join("temp1_label"), label).unwrap();
    fs::write(path.join("temp1_input"), value).unwrap();
}

#[test]
fn temperature_uses_the_hottest_cpu_package_regardless_of_hwmon_number() {
    let root = tempfile::tempdir().unwrap();
    sensor(root.path(), "hwmon2", "nvme\n", "Composite\n", "90000\n");
    sensor(
        root.path(),
        "hwmon8",
        "coretemp\n",
        "Package id 0\n",
        "65000\n",
    );
    sensor(
        root.path(),
        "hwmon12",
        "coretemp\n",
        "Package id 1\n",
        "70000\n",
    );
    fs::write(root.path().join("hwmon8/temp2_label"), "Core 0\n").unwrap();
    fs::write(root.path().join("hwmon8/temp2_input"), "95000\n").unwrap();

    assert_eq!(read_cpu_temperature(root.path()), Some(70_000));
}

#[test]
fn temperature_is_unavailable_when_cpu_readings_are_missing_or_invalid() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(read_cpu_temperature(root.path()), None);
    assert_eq!(read_cpu_temperature(&root.path().join("missing")), None);
    sensor(
        root.path(),
        "hwmon8",
        "coretemp\n",
        "Package id 0\n",
        "65000\n",
    );
    assert_eq!(read_cpu_temperature(root.path()), Some(65_000));

    for value in ["invalid", "", "-1000", "200001", "2147483648"] {
        fs::write(root.path().join("hwmon8/temp1_input"), value).unwrap();
        assert_eq!(read_cpu_temperature(root.path()), None, "{value}");
    }
    fs::remove_file(root.path().join("hwmon8/temp1_input")).unwrap();
    assert_eq!(read_cpu_temperature(root.path()), None);
}

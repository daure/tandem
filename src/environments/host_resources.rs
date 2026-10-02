use std::path::Path;

use crate::store::environments::EnvironmentSnapshot;

pub(super) struct HostResources {
    available_memory_bytes: Option<u64>,
    cpu_temperature_millicelsius: Option<i32>,
}

impl HostResources {
    pub(super) fn read() -> Self {
        let mut system = sysinfo::System::new();
        system.refresh_memory();
        let available_memory = system.available_memory();
        Self {
            available_memory_bytes: (available_memory > 0).then_some(available_memory),
            cpu_temperature_millicelsius: super::cpu_temperature::read_cpu_temperature(Path::new(
                "/sys/class/hwmon",
            )),
        }
    }

    pub(super) fn publish(self, snapshot: &mut EnvironmentSnapshot) {
        snapshot.available_memory_bytes = self.available_memory_bytes;
        snapshot.cpu_temperature_millicelsius = self.cpu_temperature_millicelsius;
        snapshot.resource_revision = Some(snapshot.resource_revision.unwrap_or_default() + 1);
    }
}

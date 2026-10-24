// SPDX-License-Identifier: GPL-3.0-only

//! System resources for the home tab: CPU, memory, disks, network, battery.
//! Everything is read from `/proc` and `/sys`, which takes microseconds.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, Once};
use std::time::{Duration, Instant};

/// Samples kept for the history graphs, one per second.
pub const HISTORY: usize = 48;

/// Time a CPU spent busy and in total, in clock ticks, from `/proc/stat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
}

pub fn parse_cpu(stat: &str) -> Option<CpuTimes> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let values: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|v| v.parse().ok())
        .collect();
    // user nice system idle iowait irq softirq steal (guest is already in user).
    let total: u64 = values.iter().take(8).sum();
    let idle = values.get(3)? + values.get(4).copied().unwrap_or(0);
    Some(CpuTimes {
        busy: total.saturating_sub(idle),
        total,
    })
}

/// Share of time the CPU was busy between two readings, 0–1.
pub fn cpu_usage(before: CpuTimes, after: CpuTimes) -> f32 {
    let total = after.total.saturating_sub(before.total);
    if total == 0 {
        return 0.0;
    }
    (after.busy.saturating_sub(before.busy) as f32 / total as f32).clamp(0.0, 1.0)
}

/// Memory in bytes, from `/proc/meminfo`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
}

impl Memory {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
}

pub fn parse_meminfo(meminfo: &str) -> Memory {
    let mut memory = Memory::default();
    for line in meminfo.lines() {
        let mut parts = line.split_whitespace();
        let (Some(key), Some(value)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Ok(kib) = value.parse::<u64>() else {
            continue;
        };
        let bytes = kib * 1024;
        match key {
            "MemTotal:" => memory.total = bytes,
            "MemAvailable:" => memory.available = bytes,
            _ => {}
        }
    }
    memory
}

/// Bytes received and sent by all interfaces but loopback, from `/proc/net/dev`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetCounters {
    pub rx: u64,
    pub tx: u64,
}

pub fn parse_net_dev(dev: &str) -> NetCounters {
    let mut counters = NetCounters::default();
    for line in dev.lines().skip(2) {
        let Some((name, values)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        // Loopback and virtual bridges would count traffic twice.
        if name == "lo" || name.starts_with("veth") || name.starts_with("docker") {
            continue;
        }
        let values: Vec<u64> = values
            .split_whitespace()
            .filter_map(|v| v.parse().ok())
            .collect();
        counters.rx += values.first().copied().unwrap_or(0);
        counters.tx += values.get(8).copied().unwrap_or(0);
    }
    counters
}

/// A mounted local filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    pub mount: String,
    pub total: u64,
    pub available: u64,
}

impl Disk {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
}

const DISK_FILESYSTEMS: &[&str] = &[
    "ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "zfs", "bcachefs", "ntfs", "ntfs3", "exfat",
    "vfat",
];

/// Mount points worth showing: local filesystems the user keeps files on.
/// Network filesystems are left out, as asking them for space can hang.
pub fn disk_mounts(mounts: &str) -> Vec<String> {
    let mut seen_devices = Vec::new();
    let mut result = Vec::new();
    for line in mounts.lines() {
        let mut parts = line.split_whitespace();
        let (Some(device), Some(mount), Some(fstype)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let mount = mount.replace("\\040", " ");
        if !DISK_FILESYSTEMS.contains(&fstype)
            || mount.starts_with("/boot")
            || mount.starts_with("/efi")
            || mount == "/recovery"
            || mount.starts_with("/snap")
            || mount.starts_with("/var/")
        {
            continue;
        }
        // Btrfs subvolumes of one device share their space; show it once.
        if seen_devices.iter().any(|d| d == device) {
            continue;
        }
        seen_devices.push(device.to_string());
        result.push(mount);
    }
    result
}

fn disk_space(mount: &str) -> Option<Disk> {
    let path = std::ffi::CString::new(mount).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is a valid C string and `stat` is written by statvfs on success.
    let stat = unsafe {
        if libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) != 0 {
            return None;
        }
        stat.assume_init()
    };
    let block = stat.f_frsize as u64;
    let total = stat.f_blocks as u64 * block;
    (total > 0).then(|| Disk {
        mount: mount.to_string(),
        total,
        available: stat.f_bavail as u64 * block,
    })
}

/// Where a Flatpak sees the host's root filesystem (with `--filesystem=host:ro`).
const HOST_ROOT: &str = "/run/host/root";

pub fn disks() -> Vec<Disk> {
    use std::os::unix::fs::MetadataExt;
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let mut paths = disk_mounts(&mounts);
    if crate::in_flatpak() {
        // The sandbox's own mounts are the runtime's; of the rest, the user's
        // disks keep their paths and the host's root is elsewhere.
        paths.retain(|m| sandbox_user_mount(m));
        paths.insert(0, HOST_ROOT.to_string());
    }
    let mut devices = Vec::new();
    paths
        .iter()
        .filter(|m| {
            // Paths of one filesystem share their space; show it once.
            let device = std::fs::metadata(m).map(|m| m.dev()).ok();
            let new = !devices.contains(&device);
            devices.push(device);
            new
        })
        .filter_map(|m| disk_space(m))
        .map(|disk| match disk.mount.as_str() {
            HOST_ROOT => Disk { mount: "/".into(), ..disk },
            _ => disk,
        })
        .collect()
}

/// Mounts of the user's disks inside a Flatpak sandbox.
fn sandbox_user_mount(mount: &str) -> bool {
    ["/home", "/media", "/mnt", "/run/media"]
        .iter()
        .any(|p| mount == *p || mount.strip_prefix(p).is_some_and(|rest| rest.starts_with('/')))
}

/// Average current clock of all CPU cores in GHz, from cpufreq.
pub fn cpu_frequency() -> Option<f32> {
    let mut sum = 0u64;
    let mut count = 0u64;
    for entry in std::fs::read_dir("/sys/devices/system/cpu").ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.strip_prefix("cpu").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
            continue;
        }
        if let Some(khz) = read_number(&entry.path().join("cpufreq/scaling_cur_freq")) {
            sum += khz;
            count += 1;
        }
    }
    (count > 0).then(|| sum as f32 / count as f32 / 1_000_000.0)
}

/// Temperature of the CPU package in °C, from hwmon.
pub fn cpu_temperature() -> Option<f32> {
    const SENSORS: &[&str] = &["coretemp", "k10temp", "zenpower", "cpu_thermal", "acpitz"];
    let mut found: Option<(usize, f32)> = None;
    for entry in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let dir = entry.path();
        let Ok(name) = std::fs::read_to_string(dir.join("name")) else {
            continue;
        };
        let Some(rank) = SENSORS.iter().position(|s| *s == name.trim()) else {
            continue;
        };
        if found.is_some_and(|(best, _)| best <= rank) {
            continue;
        }
        if let Some(milli) = read_number(&dir.join("temp1_input")) {
            found = Some((rank, milli as f32 / 1000.0));
        }
    }
    found.map(|(_, t)| t)
}

/// Which maker's driver reports the graphics card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Vendor {
    Amd,
    Intel,
}

/// A graphics card the kernel reports use of.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GpuSource {
    card: std::path::PathBuf,
    vendor: Vendor,
}

/// What the home tab shows of the graphics card.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Gpu {
    pub name: &'static str,
    pub freq_mhz: Option<u32>,
    /// Video memory used and in total, in bytes (discrete cards).
    pub vram: Option<(u64, u64)>,
    pub temperature: Option<f32>,
}

/// The first AMD or Intel card; others (NVIDIA's own driver) report nothing in sysfs.
fn find_gpu() -> Option<GpuSource> {
    let mut found: Vec<GpuSource> = Vec::new();
    for entry in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // card0, card1 … but not card1-DP-1 (its outputs).
        if !name.starts_with("card") || name[4..].parse::<u32>().is_err() {
            continue;
        }
        let card = entry.path();
        let vendor = match std::fs::read_to_string(card.join("device/vendor")) {
            Ok(v) if v.trim() == "0x1002" => Vendor::Amd,
            Ok(v) if v.trim() == "0x8086" => Vendor::Intel,
            _ => continue,
        };
        found.push(GpuSource { card, vendor });
    }
    // A discrete AMD card does the heavy work, if there is one.
    found.sort_by_key(|g| (g.vendor != Vendor::Amd, g.card.clone()));
    found.into_iter().next()
}

/// Share of time the card was busy, 0–1, from how long it rested (RC6) in `elapsed_ms`.
pub fn busy_from_rc6(before_ms: u64, after_ms: u64, elapsed_ms: f64) -> f32 {
    if elapsed_ms <= 0.0 {
        return 0.0;
    }
    (1.0 - after_ms.saturating_sub(before_ms) as f64 / elapsed_ms).clamp(0.0, 1.0) as f32
}

fn gpu_temperature(device: &Path) -> Option<f32> {
    let hwmon = std::fs::read_dir(device.join("hwmon")).ok()?.flatten().next()?.path();
    read_number(&hwmon.join("temp1_input")).map(|m| m as f32 / 1000.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charge {
    Charging,
    Discharging,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Battery {
    pub percent: u8,
    pub charge: Charge,
}

pub fn battery() -> Option<Battery> {
    for entry in std::fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let dir = entry.path();
        let kind = std::fs::read_to_string(dir.join("type")).unwrap_or_default();
        let scope = std::fs::read_to_string(dir.join("scope")).unwrap_or_default();
        // Mice and headsets report a "Device" scope.
        if kind.trim() != "Battery" || scope.trim() == "Device" {
            continue;
        }
        let Some(percent) = read_number(&dir.join("capacity")) else {
            continue;
        };
        let status = std::fs::read_to_string(dir.join("status")).unwrap_or_default();
        let charge = match status.trim() {
            "Charging" => Charge::Charging,
            "Full" | "Not charging" => Charge::Full,
            _ => Charge::Discharging,
        };
        return Some(Battery {
            percent: percent.min(100) as u8,
            charge,
        });
    }
    None
}

fn read_number(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

pub fn uptime() -> Duration {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map(Duration::from_secs_f64)
        .unwrap_or_default()
}

static MONITOR: LazyLock<Mutex<Monitor>> = LazyLock::new(Default::default);
/// The home tab is open: read everything, not just what the graphs need.
static DETAILED: AtomicBool = AtomicBool::new(false);
/// Something shows the CPU or network graphs: keep their history. Otherwise
/// the sampling thread sleeps until it is wanted, without waking up.
static HISTORY_WANTED: AtomicBool = AtomicBool::new(false);
static WAKE: (Mutex<()>, std::sync::Condvar) = (Mutex::new(()), std::sync::Condvar::new());

/// Whether the graphs of the home page are kept up to date.
pub fn set_history_wanted(wanted: bool) {
    if HISTORY_WANTED.swap(wanted, Ordering::Relaxed) != wanted && wanted {
        WAKE.1.notify_all();
    }
}

/// The home tab shows the graphics card: read it too.
static GPU_SHOWN: AtomicBool = AtomicBool::new(false);

static GPU_SOURCE: LazyLock<Option<GpuSource>> = LazyLock::new(find_gpu);

/// Whether there is a graphics card the island can read.
pub fn gpu_present() -> bool {
    GPU_SOURCE.is_some()
}

/// Reads the graphics card only while the home tab shows it.
pub fn set_gpu_shown(shown: bool) {
    GPU_SHOWN.store(shown, Ordering::Relaxed);
}

/// Starts sampling in the background, once a second. The graphs need a sample
/// every second to have a history when the tab opens, but those samples need
/// no redraw, so they are taken off the UI thread.
pub fn start() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new()
            .name("system-monitor".into())
            .spawn(|| {
                loop {
                    if !HISTORY_WANTED.load(Ordering::Relaxed) {
                        // Nothing shows the graphs: sleep until something does, and
                        // start the next history afresh.
                        let guard = WAKE.0.lock().unwrap_or_else(|e| e.into_inner());
                        let _unused = WAKE
                            .1
                            .wait_while(guard, |()| !HISTORY_WANTED.load(Ordering::Relaxed))
                            .unwrap_or_else(|e| e.into_inner());
                        let mut monitor = monitor();
                        monitor.cpu_before = None;
                        monitor.net_before = None;
                        monitor.sampled_at = None;
                    }
                    let now = Instant::now();
                    let detailed = DETAILED.load(Ordering::Relaxed);
                    let mut monitor = monitor();
                    monitor.sample_history(now);
                    if detailed {
                        monitor.sample_details(now);
                    }
                    drop(monitor);
                    std::thread::sleep(Duration::from_secs(1));
                }
            });
        if let Err(err) = spawned {
            tracing::warn!("cannot start the system monitor: {err}");
        }
    });
}

/// The latest samples.
pub fn monitor() -> MutexGuard<'static, Monitor> {
    MONITOR.lock().unwrap_or_else(|e| e.into_inner())
}

/// Reads memory, disks, battery and the rest only while the home tab shows them.
pub fn set_detailed(detailed: bool) {
    if detailed && !DETAILED.swap(true, Ordering::Relaxed) {
        // Opening the tab shows current numbers right away.
        monitor().sample_details(Instant::now());
    } else if !detailed {
        DETAILED.store(false, Ordering::Relaxed);
    }
}

/// What the home tab shows, updated about once a second.
#[derive(Debug, Default)]
pub struct Monitor {
    cpu_before: Option<CpuTimes>,
    net_before: Option<(NetCounters, Instant)>,
    sampled_at: Option<Instant>,
    slow_at: Option<Instant>,
    /// CPU use, 0–1, oldest first.
    pub cpu: VecDeque<f32>,
    /// Download and upload speed in bytes per second, oldest first.
    pub rx: VecDeque<f64>,
    pub tx: VecDeque<f64>,
    pub cores: usize,
    pub temperature: Option<f32>,
    /// Average core clock in GHz.
    pub frequency: Option<f32>,
    pub memory: Memory,
    pub disks: Vec<Disk>,
    pub battery: Option<Battery>,
    pub uptime: Duration,
    gpu_before: Option<(u64, Instant)>,
    /// The graphics card, if there is one the kernel reports on.
    pub gpu: Option<Gpu>,
    /// Graphics card use, 0–1, oldest first.
    pub gpu_busy: VecDeque<f32>,
}

fn push<T>(history: &mut VecDeque<T>, value: T) {
    if history.len() == HISTORY {
        history.pop_front();
    }
    history.push_back(value);
}

impl Monitor {
    /// Samples CPU use and network speed for the graphs, unless that was
    /// done less than a second ago.
    pub fn sample_history(&mut self, now: Instant) {
        if self
            .sampled_at
            .is_some_and(|at| now.duration_since(at) < Duration::from_millis(900))
        {
            return;
        }
        self.sampled_at = Some(now);

        if let Some(cpu) = std::fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|s| parse_cpu(&s))
        {
            if let Some(before) = self.cpu_before {
                push(&mut self.cpu, cpu_usage(before, cpu));
            }
            self.cpu_before = Some(cpu);
        }
        if let Ok(dev) = std::fs::read_to_string("/proc/net/dev") {
            let counters = parse_net_dev(&dev);
            if let Some((before, at)) = self.net_before {
                let seconds = now.duration_since(at).as_secs_f64().max(0.001);
                push(&mut self.rx, counters.rx.saturating_sub(before.rx) as f64 / seconds);
                push(&mut self.tx, counters.tx.saturating_sub(before.tx) as f64 / seconds);
            }
            self.net_before = Some((counters, now));
        }
        if GPU_SHOWN.load(Ordering::Relaxed) {
            self.sample_gpu(now);
        } else {
            // Resting: the first reading after it starts a new interval.
            self.gpu_before = None;
        }
    }

    fn sample_gpu(&mut self, now: Instant) {
        let Some(GpuSource { card, vendor }) = GPU_SOURCE.clone() else {
            return;
        };
        let device = card.join("device");
        let busy = match vendor {
            Vendor::Amd => read_number(&device.join("gpu_busy_percent")).map(|p| p as f32 / 100.0),
            Vendor::Intel => {
                let rc6 = read_number(&card.join("gt/gt0/rc6_residency_ms"));
                let busy = match (rc6, self.gpu_before) {
                    (Some(after), Some((before, at))) => Some(busy_from_rc6(
                        before,
                        after,
                        now.duration_since(at).as_secs_f64() * 1000.0,
                    )),
                    _ => None,
                };
                self.gpu_before = rc6.map(|ms| (ms, now));
                busy
            }
        };
        if let Some(busy) = busy {
            push(&mut self.gpu_busy, busy.clamp(0.0, 1.0));
        }
        let freq = match vendor {
            Vendor::Intel => read_number(&card.join("gt_act_freq_mhz")),
            Vendor::Amd => None,
        };
        self.gpu = Some(Gpu {
            name: match vendor {
                Vendor::Amd => "AMD",
                Vendor::Intel => "Intel",
            },
            freq_mhz: freq.map(|f| f as u32),
            vram: read_number(&device.join("mem_info_vram_used"))
                .zip(read_number(&device.join("mem_info_vram_total"))),
            temperature: gpu_temperature(&device),
        });
    }

    /// Reads the rest. Disks, battery and temperature change slowly and are
    /// read at most every five seconds.
    pub fn sample_details(&mut self, now: Instant) {
        if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
            self.memory = parse_meminfo(&meminfo);
        }
        self.uptime = uptime();
        self.frequency = cpu_frequency();

        if self
            .slow_at
            .is_none_or(|at| now.duration_since(at) >= Duration::from_secs(5))
        {
            self.slow_at = Some(now);
            self.cores = std::thread::available_parallelism().map_or(1, |n| n.get());
            self.temperature = cpu_temperature();
            self.disks = disks();
            self.battery = battery();
        }
    }

    pub fn cpu_now(&self) -> f32 {
        self.cpu.back().copied().unwrap_or(0.0)
    }
}

/// Languages that write decimals with a comma.
fn decimal_comma(locale: chrono::Locale) -> bool {
    const COMMA: &[&str] = &[
        "cs", "sk", "pl", "de", "fr", "es", "it", "pt", "ru", "uk", "nl", "sv", "da", "nb", "nn",
        "fi", "hu", "ro", "tr", "hr", "sl", "sr", "bg", "lt", "lv", "et", "el", "id", "be",
    ];
    let name = format!("{locale:?}");
    let language = name.split('_').next().unwrap_or_default();
    COMMA.contains(&language)
}

/// A clock speed: "1,7 GHz".
pub fn format_ghz(ghz: f32, locale: chrono::Locale) -> String {
    let text = format!("{ghz:.1}");
    let text = if decimal_comma(locale) {
        text.replace('.', ",")
    } else {
        text
    };
    format!("{text} GHz")
}

/// A size in decimal units, as COSMIC Files shows them: "7,8 GB", "512 MB".
pub fn format_bytes(bytes: f64, locale: chrono::Locale) -> String {
    const UNITS: &[&str] = &["B", "kB", "MB", "GB", "TB", "PB"];
    let mut value = bytes.max(0.0);
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    let text = if unit == 0 || value >= 100.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    };
    let text = if decimal_comma(locale) {
        text.replace('.', ",")
    } else {
        text
    };
    format!("{text} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_usage_from_stat() {
        let before = parse_cpu("cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 1 2 3 4\n").unwrap();
        let after = parse_cpu("cpu  200 0 200 1000 0 0 0 0 0 0\n").unwrap();
        assert_eq!(before, CpuTimes { busy: 200, total: 1000 });
        assert!((cpu_usage(before, after) - 0.5).abs() < 1e-6);
        assert_eq!(cpu_usage(after, after), 0.0);
    }

    #[test]
    fn meminfo() {
        let memory = parse_meminfo(
            "MemTotal:       16000000 kB\nMemFree:  1 kB\nMemAvailable:    6000000 kB\n\
             SwapTotal:       2000000 kB\nSwapFree:        1500000 kB\n",
        );
        assert_eq!(memory.used(), 10_000_000 * 1024);
    }

    #[test]
    fn user_mounts_in_a_sandbox() {
        assert!(sandbox_user_mount("/home"));
        assert!(sandbox_user_mount("/run/media/jirka/USB"));
        assert!(!sandbox_user_mount("/usr"));
        assert!(!sandbox_user_mount("/homework"));
    }

    #[test]
    fn net_dev_skips_loopback() {
        let dev = "Inter-|   Receive |  Transmit\n face |bytes packets\n\
                       lo: 500 5 0 0 0 0 0 0 500 5 0 0 0 0 0 0\n\
                   wlp0s20f3: 1000 10 0 0 0 0 0 0 300 3 0 0 0 0 0 0\n\
                     enp2s0: 24 1 0 0 0 0 0 0 6 1 0 0 0 0 0 0\n";
        assert_eq!(parse_net_dev(dev), NetCounters { rx: 1024, tx: 306 });
    }

    #[test]
    fn disk_mounts_keep_user_filesystems() {
        let mounts = "/dev/mapper/data-root / ext4 rw 0 0\n\
                      proc /proc proc rw 0 0\n\
                      /dev/nvme0n1p1 /boot/efi vfat rw 0 0\n\
                      /dev/nvme0n1p2 /recovery vfat rw 0 0\n\
                      /dev/sda1 /media/jiri/USB\\040disk exfat rw 0 0\n\
                      /dev/sdb1 /home btrfs rw 0 0\n\
                      /dev/sdb1 /home/.snapshots btrfs rw 0 0\n\
                      server:/x /mnt/nas nfs4 rw 0 0\n";
        assert_eq!(disk_mounts(mounts), ["/", "/media/jiri/USB disk", "/home"]);
    }

    #[test]
    fn bytes_by_locale() {
        let cs = chrono::Locale::cs_CZ;
        assert_eq!(format_bytes(512.0, cs), "512 B");
        assert_eq!(format_bytes(7_812_000_000.0, cs), "7,8 GB");
        assert_eq!(format_bytes(256_000_000_000.0, cs), "256 GB");
        assert_eq!(format_bytes(1_500.0, chrono::Locale::en_US), "1.5 kB");
    }

    #[test]
    fn gpu_busy_from_rest_time() {
        // Rested 730 ms of 1000 ms: busy 27 %.
        assert!((busy_from_rc6(6_000, 6_730, 1000.0) - 0.27).abs() < 1e-4);
        assert_eq!(busy_from_rc6(6_000, 7_500, 1000.0), 0.0);
        assert_eq!(busy_from_rc6(6_000, 6_000, 0.0), 0.0);
    }

    #[test]
    fn history_is_bounded() {
        let mut history = VecDeque::new();
        for i in 0..HISTORY + 5 {
            push(&mut history, i);
        }
        assert_eq!(history.len(), HISTORY);
        assert_eq!(history.front(), Some(&5));
    }
}

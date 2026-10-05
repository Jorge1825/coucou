// Read-only snapshot of the PC's resources, for the chat's `system_stats` tool.
//
// It is only ever read when the model calls the tool, i.e. when the user asked
// about their computer in the chat. Nothing polls in the background and nothing
// is stored. Deliberately coarse: load, memory, free disk space, battery,
// uptime. No process names, no file names, no network addresses — those say far
// more about what the user does than a question about speed needs.

use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

/// `DRIVE_FIXED`: local hard disks and SSDs, not USB sticks or network shares.
const DRIVE_FIXED: u32 = 3;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Disk {
    pub name: String,
    pub free_gb: f64,
    pub total_gb: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    pub on_ac: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    /// Share of CPU time busy over a short sample, 0–100.
    pub cpu_percent: Option<f64>,
    pub mem_used_gb: f64,
    pub mem_total_gb: f64,
    pub disks: Vec<Disk>,
    /// None on a desktop, or when Windows can't tell.
    pub battery: Option<Battery>,
    pub uptime_hours: f64,
}

fn ticks(t: &FILETIME) -> u64 {
    (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
}

/// CPU busy share between two instants. `kernel` already includes idle time.
fn cpu_between(a: (u64, u64, u64), b: (u64, u64, u64)) -> Option<f64> {
    let idle = b.0.checked_sub(a.0)?;
    let total = b.1.checked_sub(a.1)? + b.2.checked_sub(a.2)?;
    if total == 0 {
        return None;
    }
    Some(((total - idle.min(total)) as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
}

fn system_times() -> Option<(u64, u64, u64)> {
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()? };
    Some((ticks(&idle), ticks(&kernel), ticks(&user)))
}

fn cpu_percent() -> Option<f64> {
    let first = system_times()?;
    std::thread::sleep(Duration::from_millis(250));
    cpu_between(first, system_times()?)
}

fn memory() -> Option<(f64, f64)> {
    let mut status = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut status).ok()? };
    let total = status.ullTotalPhys as f64 / GIB;
    let avail = status.ullAvailPhys as f64 / GIB;
    Some((total - avail, total))
}

fn disks() -> Vec<Disk> {
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root: Vec<u16> = format!("{letter}:\\").encode_utf16().chain(Some(0)).collect();
        if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } != DRIVE_FIXED {
            continue;
        }
        let (mut free, mut total) = (0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), Some(&mut free), Some(&mut total), None) }.is_ok() && total > 0 {
            out.push(Disk { name: format!("{letter}:"), free_gb: free as f64 / GIB, total_gb: total as f64 / GIB });
        }
    }
    out
}

fn battery() -> Option<Battery> {
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut s).ok()? };
    // BatteryFlag 128 = no battery; 255 / percent 255 = unknown.
    if s.BatteryFlag == 128 || s.BatteryFlag == 255 || s.BatteryLifePercent > 100 {
        return None;
    }
    Some(Battery { percent: s.BatteryLifePercent, charging: s.BatteryFlag & 8 != 0, on_ac: s.ACLineStatus == 1 })
}

pub fn read() -> Stats {
    let (mem_used_gb, mem_total_gb) = memory().unwrap_or((0.0, 0.0));
    Stats {
        cpu_percent: cpu_percent(),
        mem_used_gb,
        mem_total_gb,
        disks: disks(),
        battery: battery(),
        uptime_hours: unsafe { GetTickCount64() } as f64 / 3_600_000.0,
    }
}

/// The text the model gets back. Plain facts with units, nothing editorial.
pub fn describe(s: &Stats) -> String {
    let mut out = Vec::new();
    match s.cpu_percent {
        Some(p) => out.push(format!("CPU: {p:.0}% busy")),
        None => out.push("CPU: unavailable".into()),
    }
    if s.mem_total_gb > 0.0 {
        out.push(format!(
            "Memory: {:.1} of {:.1} GB in use ({:.0}%)",
            s.mem_used_gb,
            s.mem_total_gb,
            s.mem_used_gb / s.mem_total_gb * 100.0
        ));
    }
    for d in &s.disks {
        out.push(format!("Disk {} {:.0} GB free of {:.0} GB", d.name, d.free_gb, d.total_gb));
    }
    match &s.battery {
        Some(b) => out.push(format!(
            "Battery: {}%{}",
            b.percent,
            if b.charging { ", charging" } else if b.on_ac { ", plugged in" } else { ", on battery" }
        )),
        None => out.push("Battery: none (desktop) or unknown".into()),
    }
    out.push(if s.uptime_hours >= 48.0 {
        format!("Uptime: {:.1} days", s.uptime_hours / 24.0)
    } else {
        format!("Uptime: {:.1} hours", s.uptime_hours)
    });
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_share_is_busy_time_over_total_time() {
        // 1000 ticks passed (kernel includes idle): 250 idle → 75% busy.
        assert_eq!(cpu_between((0, 0, 0), (250, 600, 400)), Some(75.0));
        // Nothing elapsed, or a counter that went backwards: no number rather than a wrong one.
        assert_eq!(cpu_between((5, 5, 5), (5, 5, 5)), None);
        assert_eq!(cpu_between((9, 9, 9), (1, 1, 1)), None);
        // Idle can't exceed the total.
        assert_eq!(cpu_between((0, 0, 0), (900, 100, 100)), Some(0.0));
    }

    #[test]
    fn the_description_states_units_and_handles_missing_parts() {
        let s = Stats {
            cpu_percent: Some(42.4),
            mem_used_gb: 9.6,
            mem_total_gb: 16.0,
            disks: vec![Disk { name: "C:".into(), free_gb: 120.4, total_gb: 476.0 }],
            battery: Some(Battery { percent: 80, charging: false, on_ac: false }),
            uptime_hours: 5.5,
        };
        let text = describe(&s);
        assert!(text.contains("CPU: 42% busy"));
        assert!(text.contains("Memory: 9.6 of 16.0 GB in use (60%)"));
        assert!(text.contains("Disk C: 120 GB free of 476 GB"));
        assert!(text.contains("Battery: 80%, on battery"));
        assert!(text.contains("Uptime: 5.5 hours"));

        let bare = Stats { cpu_percent: None, mem_used_gb: 0.0, mem_total_gb: 0.0, disks: vec![], battery: None, uptime_hours: 100.0 };
        let text = describe(&bare);
        assert!(text.contains("CPU: unavailable"));
        assert!(!text.contains("Memory"));
        assert!(text.contains("Battery: none"));
        assert!(text.contains("Uptime: 4.2 days"));
    }

    #[test]
    fn this_machine_gives_plausible_numbers() {
        let s = read();
        assert!(s.mem_total_gb > 0.5, "total memory {}", s.mem_total_gb);
        assert!(s.mem_used_gb > 0.0 && s.mem_used_gb <= s.mem_total_gb);
        assert!(s.cpu_percent.is_some_and(|p| (0.0..=100.0).contains(&p)));
        assert!(!s.disks.is_empty(), "at least the system drive");
        assert!(s.disks.iter().all(|d| d.free_gb <= d.total_gb));
        assert!(s.uptime_hours > 0.0);
        println!("{}", describe(&s)); // visible with --nocapture
    }
}

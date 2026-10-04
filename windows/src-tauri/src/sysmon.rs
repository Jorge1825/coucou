// The machine at a glance: battery, CPU, memory, internet, how long since the
// last keystroke, and whether a game is running full screen.
//
// Read every few seconds from Windows' own counters — no I/O, no network, no
// process list. The island decides what is worth saying (see island/system.ts);
// this only reports.

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::Networking::NetworkListManager::{INetworkListManager, NetworkListManager};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::{GetTickCount, GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_RUNNING_D3D_FULL_SCREEN};

use crate::island;

const EVERY: Duration = Duration::from_secs(5);
/// Nothing uses the readings: look again only to notice that changing.
const IDLE_EVERY: Duration = Duration::from_secs(30);

#[derive(Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    /// Percent, or None on a machine without a battery.
    pub battery: Option<u8>,
    pub charging: bool,
    /// Percent of all cores over the last interval.
    pub cpu: u8,
    /// Percent of physical memory in use.
    pub memory: u8,
    pub online: bool,
    /// Seconds since the last keyboard or mouse input.
    pub idle_secs: u32,
    /// A Direct3D game is running full screen.
    pub game: bool,
}

/// Something on the island wants these readings.
fn wanted(app: &AppHandle) -> bool {
    app.try_state::<crate::Shared>()
        .map(|s| {
            let s = s.settings.lock().unwrap();
            s.system.enabled || s.day.enabled || (s.outfits.auto && s.outfits.gamer)
        })
        .unwrap_or(false)
}

fn ft(t: FILETIME) -> u64 {
    ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
}

/// (idle, total) CPU ticks since boot.
fn cpu_ticks() -> Option<(u64, u64)> {
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()? };
    // Kernel time includes idle time.
    Some((ft(idle), ft(kernel) + ft(user)))
}

fn battery() -> (Option<u8>, bool) {
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() {
        return (None, false);
    }
    // 128 = no system battery, 255 = unknown.
    let present = s.BatteryFlag & 128 == 0 && s.BatteryLifePercent <= 100;
    (present.then_some(s.BatteryLifePercent), s.ACLineStatus == 1)
}

fn memory() -> u8 {
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut m) }.is_err() {
        return 0;
    }
    m.dwMemoryLoad.min(100) as u8
}

fn idle_secs() -> u32 {
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return 0;
    }
    let now = unsafe { GetTickCount() };
    now.wrapping_sub(info.dwTime) / 1000
}

fn game_full_screen() -> bool {
    unsafe { SHQueryUserNotificationState() }
        .map(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN)
        .unwrap_or(false)
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        // The network list manager is a COM object; this thread is its apartment.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let network: Option<INetworkListManager> =
            unsafe { CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL) }.ok();
        let mut last_cpu = cpu_ticks();
        loop {
            let on = wanted(&app) && !crate::integrations::PAUSED.load(Ordering::Relaxed);
            std::thread::sleep(if on { EVERY } else { IDLE_EVERY });
            if !on {
                last_cpu = cpu_ticks();
                continue;
            }
            let now_cpu = cpu_ticks();
            let cpu = match (last_cpu, now_cpu) {
                (Some((i0, t0)), Some((i1, t1))) if t1 > t0 => {
                    let busy = (t1 - t0).saturating_sub(i1 - i0);
                    ((busy * 100) / (t1 - t0)).min(100) as u8
                }
                _ => 0,
            };
            last_cpu = now_cpu;
            let (battery, charging) = battery();
            let online = network
                .as_ref()
                .and_then(|n| unsafe { n.IsConnectedToInternet() }.ok())
                .map(|b| b.as_bool())
                .unwrap_or(true);
            let status = SystemStatus {
                battery,
                charging,
                cpu,
                memory: memory(),
                online,
                idle_secs: idle_secs(),
                game: game_full_screen(),
            };
            island::emit_all(&app, "system-status", status);
        }
    });
}


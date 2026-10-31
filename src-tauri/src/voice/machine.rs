//! A quick look at this machine to recommend the speech model that fits.
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Machine {
    pub cpu: String,
    pub cores: u32,
    pub ram_gb: f64,
    /// The graphics card, by name, and its own memory in GB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<(String, f64)>,
    /// That card shares the computer's memory (built into the processor).
    pub gpu_integrated: bool,
    /// A Mac, where the GPU engine runs on Metal; on Apple silicon its memory is shared.
    pub mac: bool,
    pub apple_silicon: bool,
    /// The GPU helper is published for this computer.
    pub helper: bool,
    pub score: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_free_gb: Option<f64>,
}

pub fn probe(ms: u64) -> u64 {
    let t = std::time::Instant::now();
    let (mut x, mut n) = (1.000_001f64, 0u64);
    while t.elapsed().as_millis() < ms as u128 {
        for _ in 0..1000 {
            x = x * 1.000_000_1 + 1e-9;
        }
        n += 1;
    }
    std::hint::black_box(x);
    n
}

/// The recommendation for this machine, measured once per launch.
pub fn recommended() -> &'static str {
    static R: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    R.get_or_init(|| recommend(&measure()))
}

/// Which model this machine should start with: Whisper Turbo on the graphics card where one can run it, else Moonshine.
pub fn recommend(m: &Machine) -> &'static str {
    if m.helper && gpu_fits(m) {
        "whisper-turbo"
    } else if m.cores >= 4 && m.ram_gb >= 7.5 {
        "moonshine-base-en"
    } else {
        "moonshine-tiny-en"
    }
}

/// Whether this computer's graphics card can run Whisper Turbo well, judged without starting the helper: Apple silicon
/// with 8 GB or more; elsewhere a card with 3 GB of its own memory, or one built in with 8 GB of memory in the computer.
pub fn gpu_fits(m: &Machine) -> bool {
    if m.mac {
        return m.apple_silicon && m.ram_gb >= 7.5;
    }
    match &m.gpu {
        Some((_, vram)) if !m.gpu_integrated => *vram >= 2.9,
        Some(_) => m.ram_gb >= 7.5,
        None => false,
    }
}

/// The graphics card a description names, by vendor id: NVIDIA and AMD cards with their own memory (`vram`, bytes,
/// when known), Intel's built in. None for anything else, such as a software renderer.
pub fn card(vendor: u32, name: &str, vram: Option<u64>) -> Option<(String, f64, bool)> {
    let gb = |b: u64| (b as f64 / (1u64 << 30) as f64 * 10.0).round() / 10.0;
    let own = vram.map(gb);
    match vendor {
        0x10de => Some((name.into(), own.unwrap_or(4.0), own.is_some_and(|g| g < 1.0))),
        0x1002 => Some((name.into(), own.unwrap_or(0.0), own.is_none_or(|g| g < 1.0))),
        0x8086 => Some((name.into(), own.unwrap_or(0.0), true)),
        _ => None,
    }
}

/// The graphics card with the most memory of its own, through DXGI: the Microsoft Basic Render Driver and software
/// adapters do not count.
#[cfg(windows)]
fn windows_gpu() -> Option<(String, f64, bool)> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    let mut best: Option<(String, f64, bool)> = None;
    unsafe {
        let f: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut i = 0;
        while let Ok(a) = f.EnumAdapters1(i) {
            i += 1;
            let Ok(d) = a.GetDesc1() else { continue };
            if d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 || (d.VendorId == 0x1414 && d.DeviceId == 0x8c) {
                continue;
            }
            let n = d.Description.iter().position(|c| *c == 0).unwrap_or(d.Description.len());
            let name = String::from_utf16_lossy(&d.Description[..n]).trim().to_string();
            let Some(c) = card(d.VendorId, &name, Some(d.DedicatedVideoMemory as u64)) else { continue };
            if best.as_ref().is_none_or(|b| c.1 > b.1) {
                best = Some(c);
            }
        }
    }
    best
}

#[cfg(windows)]
fn windows_ram_gb() -> Option<f64> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut st = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut st) }.ok()?;
    Some((st.ullTotalPhys as f64 / (1u64 << 30) as f64 * 10.0).round() / 10.0)
}

/// The graphics card the kernel lists under /sys/class/drm, best effort.
#[cfg(target_os = "linux")]
fn linux_gpu() -> Option<(String, f64, bool)> {
    let mut best: Option<(String, f64, bool)> = None;
    for e in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if !n.starts_with("card") || n.contains('-') {
            continue;
        }
        let dev = e.path().join("device");
        let read = |f: &str| std::fs::read_to_string(dev.join(f)).unwrap_or_default();
        let Ok(vendor) = u32::from_str_radix(read("vendor").trim().trim_start_matches("0x"), 16) else { continue };
        let vram = read("mem_info_vram_total").trim().parse::<u64>().ok();
        let name = match vendor {
            0x10de => "NVIDIA",
            0x1002 => "AMD",
            _ => "Intel",
        };
        if let Some(c) = card(vendor, name, vram).filter(|c| best.as_ref().is_none_or(|b| c.1 > b.1)) {
            best = Some(c);
        }
    }
    best
}

#[cfg(target_os = "macos")]
fn mac_ram_gb() -> f64 {
    let (mut v, mut len) = (0u64, std::mem::size_of::<u64>());
    let name = std::ffi::CString::new("hw.memsize").unwrap_or_default();
    unsafe { libc::sysctlbyname(name.as_ptr(), &mut v as *mut u64 as *mut libc::c_void, &mut len, std::ptr::null_mut(), 0) };
    (v as f64 / (1u64 << 30) as f64 * 10.0).round() / 10.0
}

pub fn estimate(bench_ms: u64, ref_score: u64, score: u64) -> u64 {
    if score == 0 || ref_score == 0 {
        return bench_ms;
    }
    bench_ms * ref_score / score
}

pub fn route<'a>(chosen: &str, recommended: &str, installed: &[&'a str]) -> Option<&'a str> {
    let pick = |id: &str| installed.iter().copied().find(|x| *x == id);
    if chosen != "auto" {
        if let Some(x) = pick(chosen) {
            return Some(x);
        }
    }
    pick(recommended).or_else(|| installed.first().copied())
}

pub fn measure() -> Machine {
    let helper = super::engine::helper_available(&super::models::catalog());
    #[cfg(target_os = "linux")]
    {
        let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
        let gpu = linux_gpu();
        return Machine {
            cpu: parse_cpuinfo(&read("/proc/cpuinfo")),
            cores: std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(1),
            ram_gb: parse_meminfo(&read("/proc/meminfo")),
            score: probe(100),
            disk_free_gb: disk_free(std::path::Path::new("/")),
            gpu_integrated: gpu.as_ref().is_some_and(|g| g.2),
            gpu: gpu.map(|g| (g.0, g.1)),
            helper,
            ..Machine::default()
        };
    }
    #[cfg(not(target_os = "linux"))]
    {
        #[allow(unused_mut)]
        let mut m = Machine {
            cores: std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(1),
            ram_gb: 8.0,
            score: probe(100),
            helper,
            ..Machine::default()
        };
        #[cfg(windows)]
        {
            m.ram_gb = windows_ram_gb().unwrap_or(m.ram_gb);
            if let Some(g) = windows_gpu() {
                m.gpu_integrated = g.2;
                m.gpu = Some((g.0, g.1));
            }
        }
        #[cfg(target_os = "macos")]
        {
            (m.mac, m.apple_silicon, m.ram_gb) = (true, cfg!(target_arch = "aarch64"), mac_ram_gb());
        }
        m
    }
}

pub fn parse_cpuinfo(s: &str) -> String {
    s.lines()
        .find_map(|l| {
            l.strip_prefix("model name")
                .and_then(|r| r.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap_or_default()
}

pub fn parse_meminfo(s: &str) -> f64 {
    s.lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|r| r.split_whitespace().next()?.parse::<f64>().ok())
        .map(|kb| (kb / (1u64 << 20) as f64 * 10.0).round() / 10.0)
        .unwrap_or(0.0)
}

pub fn parse_df(out: &str) -> Option<f64> {
    let kb = out
        .lines()
        .nth(1)?
        .split_whitespace()
        .nth(3)?
        .parse::<f64>()
        .ok()?;
    Some((kb / (1u64 << 20) as f64 * 10.0).floor() / 10.0)
}

pub fn disk_free(dir: &std::path::Path) -> Option<f64> {
    if cfg!(windows) {
        return None;
    }
    let mut d = dir;
    while !d.exists() {
        d = d.parent()?;
    }
    let o = std::process::Command::new("df")
        .arg("-Pk")
        .arg(d)
        .output()
        .ok()?;
    parse_df(&String::from_utf8_lossy(&o.stdout))
}

pub fn parse_nvidia(out: &str) -> Option<(String, f64)> {
    let line = out.lines().next()?;
    let (name, mib) = line.rsplit_once(',')?;
    let gib = mib.trim().parse::<f64>().ok()? / 1024.0;
    Some((name.trim().to_string(), (gib * 10.0).round() / 10.0))
}

pub fn view(m: &Machine, catalog_ref: &Value, models: &[(String, u64)]) -> Value {
    let ref_score = catalog_ref["score"].as_u64().unwrap_or(0);
    serde_json::json!({
        "machine": m,
        "recommended": recommend(m),
        "estimate_clip5_ms": models.iter().map(|(id, b)| (id.clone(), serde_json::json!(estimate(*b, ref_score, m.score)))).collect::<serde_json::Map<_, _>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(cores: u32, ram_gb: f64) -> Machine {
        Machine {
            cores,
            ram_gb,
            ..Machine::default()
        }
    }

    #[test]
    fn the_recommendation_follows_the_machine() {
        assert_eq!(recommend(&m(8, 8.0)), "moonshine-base-en");
        assert_eq!(recommend(&m(4, 4.0)), "moonshine-tiny-en");
        assert_eq!(recommend(&m(2, 32.0)), "moonshine-tiny-en");
    }

    #[test]
    fn whisper_turbo_on_the_graphics_card_is_recommended_where_one_can_run_it() {
        let pc = |gpu: Option<(&str, f64)>, integrated: bool, ram_gb: f64| Machine {
            cores: 8,
            ram_gb,
            gpu: gpu.map(|(n, g)| (n.to_string(), g)),
            gpu_integrated: integrated,
            helper: true,
            ..Machine::default()
        };
        assert_eq!(recommend(&pc(Some(("NVIDIA GeForce RTX 2060 SUPER", 8.0)), false, 16.0)), "whisper-turbo");
        assert_eq!(recommend(&pc(Some(("NVIDIA GeForce GTX 1060 3GB", 3.0)), false, 8.0)), "whisper-turbo");
        assert_eq!(recommend(&pc(Some(("NVIDIA GeForce GT 1030", 2.0)), false, 16.0)), "moonshine-base-en", "too little memory of its own");
        assert_eq!(recommend(&pc(Some(("Intel(R) Iris(R) Xe Graphics", 0.1)), true, 15.8)), "whisper-turbo");
        assert_eq!(recommend(&pc(Some(("Intel(R) UHD Graphics", 0.1)), true, 4.0)), "moonshine-tiny-en", "built in, with little memory");
        assert_eq!(recommend(&pc(None, false, 32.0)), "moonshine-base-en", "no graphics card");
        assert_eq!(recommend(&Machine { helper: false, ..pc(Some(("RTX 4090", 24.0)), false, 64.0) }), "moonshine-base-en", "no helper for it");
        let mac = |apple_silicon: bool, ram_gb: f64| Machine { mac: true, apple_silicon, ..pc(None, true, ram_gb) };
        assert_eq!(recommend(&mac(true, 8.0)), "whisper-turbo");
        assert_eq!(recommend(&mac(true, 4.0)), "moonshine-tiny-en", "Apple silicon with too little memory");
        assert_eq!(recommend(&mac(false, 16.0)), "moonshine-base-en", "an Intel Mac");
        // Read from what the system says about each card.
        assert_eq!(card(0x10de, "NVIDIA GeForce RTX 2060 SUPER", Some(8 << 30)), Some(("NVIDIA GeForce RTX 2060 SUPER".into(), 8.0, false)));
        assert_eq!(card(0x8086, "Intel(R) UHD Graphics 630", Some(128 << 20)), Some(("Intel(R) UHD Graphics 630".into(), 0.1, true)));
        assert_eq!(card(0x1002, "AMD Radeon(TM) Graphics", Some(512 << 20)).map(|c| c.2), Some(true), "an AMD processor's own");
        assert_eq!(card(0x1002, "AMD Radeon RX 6600", Some(8 << 30)).map(|c| c.2), Some(false));
        assert_eq!(card(0x1414, "Microsoft Basic Render Driver", Some(0)), None);
    }

    #[test]
    fn this_computer_is_measured_without_starting_anything() {
        let m = measure();
        assert!(m.cores >= 1 && m.ram_gb > 0.0);
        assert_eq!(m.mac, cfg!(target_os = "macos"));
        eprintln!("MACHINE {m:?} recommended={}", recommend(&m));
    }

    #[test]
    fn route_picks_the_chosen_model_if_installed_else_the_recommended() {
        let here = ["moonshine-tiny-en", "moonshine-base-en"];
        assert_eq!(
            route("moonshine-base-en", "moonshine-tiny-en", &here),
            Some("moonshine-base-en")
        );
        assert_eq!(
            route("auto", "moonshine-base-en", &here),
            Some("moonshine-base-en")
        );
        assert_eq!(
            route("missing", "moonshine-tiny-en", &here),
            Some("moonshine-tiny-en")
        );
        assert_eq!(route("auto", "moonshine-tiny-en", &[]), None);
    }

    #[test]
    fn meminfo_and_cpuinfo_parse() {
        assert_eq!(
            parse_cpuinfo("processor\t: 0\nmodel name\t: Test CPU\n"),
            "Test CPU"
        );
        assert!((parse_meminfo("MemTotal:     16384000 kB\n").abs() - 15.6).abs() < 0.2);
        assert_eq!(
            parse_nvidia("NVIDIA GeForce RTX 3080, 10240"),
            Some(("NVIDIA GeForce RTX 3080".into(), 10.0))
        );
    }
}

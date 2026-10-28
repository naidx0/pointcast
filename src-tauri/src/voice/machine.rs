//! A quick look at this machine to recommend the speech model that fits.
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Machine {
    pub cpu: String,
    pub cores: u32,
    pub ram_gb: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<(String, f64)>,
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

/// Which model this machine should start with.
pub fn recommend(m: &Machine) -> &'static str {
    if m.cores >= 4 && m.ram_gb >= 7.5 {
        "moonshine-base-en"
    } else {
        "moonshine-tiny-en"
    }
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
    #[cfg(target_os = "linux")]
    {
        let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
        return Machine {
            cpu: parse_cpuinfo(&read("/proc/cpuinfo")),
            cores: std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(1),
            ram_gb: parse_meminfo(&read("/proc/meminfo")),
            score: probe(100),
            disk_free_gb: disk_free(std::path::Path::new("/")),
            ..Machine::default()
        };
    }
    #[cfg(not(target_os = "linux"))]
    {
        Machine {
            cores: std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(1),
            ram_gb: 8.0,
            score: probe(100),
            ..Machine::default()
        }
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

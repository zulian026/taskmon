//! Live system sampling: processes, CPU, memory, disks, network.
//! One sampler thread refreshes a `sysinfo::System` every second and keeps
//! a 120-sample history plus per-second I/O rates computed from deltas.

use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use sysinfo::{
    Disks, Networks, Pid, Process, ProcessesToUpdate, Signal, System, Users,
};

#[derive(Debug, Clone, Serialize)]
pub struct ProcessView {
    pub pid: u32,
    pub name: String,
    pub user: String,
    pub cpu: f32,
    pub mem_bytes: u64,
    pub mem_pct: f32,
    pub status: String,
    pub parent: Option<u32>,
    pub run_time: u64,
    pub disk_read: u64,
    pub disk_write: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskView {
    pub name: String,
    pub mount: String,
    pub fs: String,
    pub total: u64,
    pub available: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetView {
    pub name: String,
    pub rx_bps: u64,
    pub tx_bps: u64,
    pub rx_total: u64,
    pub tx_total: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryPoint {
    pub t: i64,
    pub cpu: f32,
    pub mem_pct: f32,
    pub net_rx: u64,
    pub net_tx: u64,
    pub disk_r: u64,
    pub disk_w: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub timestamp_ms: i64,
    pub cpu_total: f32,
    pub cpu_per_core: Vec<f32>,
    pub cpu_count: usize,
    pub mem_total: u64,
    pub mem_used: u64,
    pub mem_free: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub load_avg: [f64; 3],
    pub uptime_secs: u64,
    pub processes: Vec<ProcessView>,
    pub disks: Vec<DiskView>,
    pub networks: Vec<NetView>,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    pub history: Vec<HistoryPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessDetail {
    pub pid: u32,
    pub name: String,
    pub user: String,
    pub cpu: f32,
    pub mem_bytes: u64,
    pub status: String,
    pub parent: Option<u32>,
    pub run_time: u64,
    pub start_time: u64,
    pub exe: Option<String>,
    pub cmdline: String,
    pub cwd: Option<String>,
    pub disk_read_total: u64,
    pub disk_write_total: u64,
}

pub struct Sampler {
    sys: System,
    networks: Networks,
    disks: Disks,
    users: Users,
    history: VecDeque<HistoryPoint>,
    prev_net: HashMap<String, (u64, u64)>,
    prev_disk: Option<(u64, u64)>,
    prev_at: Option<Instant>,
}

impl Sampler {
    pub fn new() -> Self {
        Self {
            sys: System::new_all(),
            networks: Networks::new_with_refreshed_list(),
            disks: Disks::new_with_refreshed_list(),
            users: Users::new_with_refreshed_list(),
            history: VecDeque::with_capacity(121),
            prev_net: HashMap::new(),
            prev_disk: None,
            prev_at: None,
        }
    }

    pub fn sample(&mut self) -> Snapshot {
        let now = Instant::now();
        let elapsed = self.prev_at.map(|p| now.duration_since(p).as_secs_f64().max(0.001));
        self.prev_at = Some(now);
        let ts = chrono::Utc::now().timestamp_millis();

        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();
        self.sys.refresh_processes(ProcessesToUpdate::All, true);
        self.networks.refresh(false);
        self.disks.refresh(false);

        let cpu_total = self.sys.global_cpu_usage();
        let cpu_per_core: Vec<f32> = self.sys.cpus().iter().map(|c| c.cpu_usage()).collect();
        let mem_total = self.sys.total_memory().max(1);
        let mem_used = self.sys.used_memory();
        let mem_pct = mem_used as f32 / mem_total as f32 * 100.0;
        let load = System::load_average();

        let mut processes: Vec<ProcessView> = self
            .sys
            .processes()
            .values()
            .map(|p| process_view(p, &self.users, mem_total))
            .collect();
        processes.sort_by(|a, b| {
            b.cpu
                .partial_cmp(&a.cpu)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.mem_bytes.cmp(&a.mem_bytes))
        });

        let disks: Vec<DiskView> = self
            .disks
            .list()
            .iter()
            .map(|d| DiskView {
                name: d.name().to_string_lossy().into_owned(),
                mount: d.mount_point().to_string_lossy().into_owned(),
                fs: d.file_system().to_string_lossy().into_owned(),
                total: d.total_space(),
                available: d.available_space(),
            })
            .collect();

        // Network rates from totals delta.
        let mut networks: Vec<NetView> = Vec::new();
        let mut net_rx = 0u64;
        let mut net_tx = 0u64;
        for (name, data) in self.networks.iter() {
            let (rx_total, tx_total) = (data.total_received(), data.total_transmitted());
            let (rx_bps, tx_bps) = match (self.prev_net.get(name), elapsed) {
                (Some((prx, ptx)), Some(secs)) => (
                    rx_total.saturating_sub(*prx) as f64 / secs,
                    tx_total.saturating_sub(*ptx) as f64 / secs,
                ),
                _ => (0.0, 0.0),
            };
            self.prev_net.insert(name.clone(), (rx_total, tx_total));
            // Skip loopback and empty virtual interfaces in the totals.
            if name != "lo" {
                net_rx += rx_bps as u64;
                net_tx += tx_bps as u64;
            }
            networks.push(NetView {
                name: name.clone(),
                rx_bps: rx_bps as u64,
                tx_bps: tx_bps as u64,
                rx_total,
                tx_total,
            });
        }
        networks.sort_by(|a, b| (b.rx_bps + b.tx_bps).cmp(&(a.rx_bps + a.tx_bps)));

        // Whole-disk I/O from /proc/diskstats (sectors are 512 bytes).
        let (disk_r_total, disk_w_total) = read_diskstats_totals();
        let (disk_read_bps, disk_write_bps) = match (self.prev_disk, elapsed) {
            (Some((pr, pw)), Some(secs)) => (
                (disk_r_total.saturating_sub(pr) as f64 / secs) as u64,
                (disk_w_total.saturating_sub(pw) as f64 / secs) as u64,
            ),
            _ => (0, 0),
        };
        self.prev_disk = Some((disk_r_total, disk_w_total));

        self.history.push_back(HistoryPoint {
            t: ts,
            cpu: cpu_total,
            mem_pct,
            net_rx,
            net_tx,
            disk_r: disk_read_bps,
            disk_w: disk_write_bps,
        });
        while self.history.len() > 120 {
            self.history.pop_front();
        }

        Snapshot {
            timestamp_ms: ts,
            cpu_total,
            cpu_per_core,
            cpu_count: self.sys.cpus().len(),
            mem_total,
            mem_used,
            mem_free: self.sys.free_memory(),
            swap_total: self.sys.total_swap(),
            swap_used: self.sys.used_swap(),
            load_avg: [load.one, load.five, load.fifteen],
            uptime_secs: System::uptime(),
            processes,
            disks,
            networks,
            disk_read_bps,
            disk_write_bps,
            history: self.history.iter().cloned().collect(),
        }
    }
}

fn process_view(p: &Process, users: &Users, mem_total: u64) -> ProcessView {
    let user = p
        .user_id()
        .and_then(|uid| users.get_user_by_id(uid))
        .map(|u| u.name().to_string())
        .unwrap_or_else(|| "—".to_string());
    let du = p.disk_usage();
    ProcessView {
        pid: p.pid().as_u32(),
        name: p.name().to_string_lossy().into_owned(),
        user,
        cpu: p.cpu_usage(),
        mem_bytes: p.memory(),
        mem_pct: p.memory() as f32 / mem_total as f32 * 100.0,
        status: p.status().to_string(),
        parent: p.parent().map(|pp| pp.as_u32()),
        run_time: p.run_time(),
        disk_read: du.total_read_bytes,
        disk_write: du.total_written_bytes,
    }
}

/// Kill a process: SIGTERM for a normal End task, SIGKILL when forced.
pub fn kill_process(pid: u32, force: bool) -> Result<bool, String> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let Some(p) = sys.process(Pid::from_u32(pid)) else {
        return Err(format!("no process with PID {pid}"));
    };
    let ok = if force {
        p.kill_with(Signal::Kill).unwrap_or(false)
    } else {
        p.kill()
    };
    if ok {
        Ok(true)
    } else {
        Err(format!("failed to signal PID {pid}"))
    }
}

/// Detail for one process, looked up fresh.
pub fn process_detail(pid: u32) -> Result<ProcessDetail, String> {
    let mut sys = System::new_all();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let users = Users::new_with_refreshed_list();
    let Some(p) = sys.process(Pid::from_u32(pid)) else {
        return Err(format!("no process with PID {pid}"));
    };
    let user = p
        .user_id()
        .and_then(|uid| users.get_user_by_id(uid))
        .map(|u| u.name().to_string())
        .unwrap_or_else(|| "—".to_string());
    let du = p.disk_usage();
    Ok(ProcessDetail {
        pid,
        name: p.name().to_string_lossy().into_owned(),
        user,
        cpu: p.cpu_usage(),
        mem_bytes: p.memory(),
        status: p.status().to_string(),
        parent: p.parent().map(|pp| pp.as_u32()),
        run_time: p.run_time(),
        start_time: p.start_time(),
        exe: p.exe().map(|e| e.to_string_lossy().into_owned()),
        cmdline: p
            .cmd()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" "),
        cwd: p.cwd().map(|c| c.to_string_lossy().into_owned()),
        disk_read_total: du.total_read_bytes,
        disk_write_total: du.total_written_bytes,
    })
}

/// Sum of sectors read/written over whole disks in /proc/diskstats.
/// Returns (read_bytes, written_bytes). Partitions, loop and ram devices
/// are skipped so a disk is not counted twice.
pub fn read_diskstats_totals() -> (u64, u64) {
    let Ok(text) = std::fs::read_to_string("/proc/diskstats") else {
        return (0, 0);
    };
    parse_diskstats(&text)
}

pub fn parse_diskstats(text: &str) -> (u64, u64) {
    let mut read_sectors = 0u64;
    let mut write_sectors = 0u64;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 14 {
            continue;
        }
        let name = f[2];
        if !is_whole_disk(name) {
            continue;
        }
        read_sectors += f[5].parse::<u64>().unwrap_or(0);
        write_sectors += f[9].parse::<u64>().unwrap_or(0);
    }
    (read_sectors * 512, write_sectors * 512)
}

fn is_whole_disk(name: &str) -> bool {
    if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("dm-") {
        return false;
    }
    if let Some(rest) = name.strip_prefix("nvme") {
        // nvme0n1 is a disk, nvme0n1p1 is a partition.
        return !rest.contains('p');
    }
    if let Some(rest) = name.strip_prefix("mmcblk") {
        return !rest.contains('p');
    }
    // sda, vda, hda, xvda: prefix + letters only; sda1 is a partition.
    for prefix in ["sd", "vd", "hd", "xvd"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_lowercase());
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diskstats_parses_whole_disks_only() {
        let text = "   8       0 sda 100 0 8000 0 50 0 4000 0 0 0 0 0 0\n   8       1 sda1 10 0 800 0 5 0 400 0 0 0 0 0 0\n 259       0 nvme0n1 20 0 1600 0 10 0 800 0 0 0 0 0 0\n 259       1 nvme0n1p1 2 0 160 0 1 0 80 0 0 0 0 0 0\n   7       0 loop0 0 0 0 0 0 0 0 0 0 0 0 0 0\n";
        let (r, w) = parse_diskstats(text);
        // sda 8000 + nvme0n1 1600 sectors read; 4000 + 800 written.
        assert_eq!(r, (8000 + 1600) * 512);
        assert_eq!(w, (4000 + 800) * 512);
    }

    #[test]
    fn diskstats_empty_is_zero() {
        assert_eq!(parse_diskstats(""), (0, 0));
    }
}

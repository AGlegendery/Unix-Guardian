//! `ug-stress`: a GPU stability test.
//!
//! A compute shader churns integers (multiply, xor, shift) while reading
//! scattered words from a large buffer, so both the cores and the memory are
//! busy. Integer arithmetic is exact, so the CPU can compute the same answer:
//! after every round a sample of results is checked, and a single wrong word
//! means the GPU is not stable at its current clocks. A lost device (the
//! driver reset the GPU) also counts as a failure.
//!
//! Prints one JSON line per second and a final verdict; exit code 0 stable,
//! 2 unstable, 1 could not run.

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "ug-stress", about = "GPU stability test (compute, self-checking)")]
struct Args {
    /// Seconds to run.
    #[arg(long, default_value_t = 120)]
    seconds: u64,
    /// PCI address of the GPU (e.g. 0000:01:00.0); default the discrete one.
    #[arg(long)]
    pci: Option<String>,
    /// Megabytes of GPU memory to churn.
    #[arg(long, default_value_t = 64)]
    megabytes: u32,
    /// List the GPUs and exit.
    #[arg(long)]
    list: bool,
    /// Check against a wrong answer on purpose: proves a fault is caught.
    #[arg(long, hide = true)]
    inject_fault: bool,
}

/// Rounds aim at this long, so a slow GPU still gets checked often.
const ROUND: Duration = Duration::from_millis(400);
const SHADER: &str = r#"
@group(0) @binding(0) var<storage, read> src: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;
@group(0) @binding(2) var<uniform> params: vec4<u32>; // seed, len, iters, 0

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>) {
    let i = id.x + id.y * nw.x * 256u;
    let len = params.y;
    if (i >= len) { return; }
    var x = src[i] ^ params.x;
    for (var k = 0u; k < params.z; k = k + 1u) {
        x = x * 1664525u + 1013904223u;
        x = x ^ (x >> 13u);
        x = x * 0x5bd1e995u;
        x = x ^ src[x % len];
    }
    dst[i] = x;
}
"#;

/// The same computation on the CPU, for one index.
fn reference(src: &[u32], seed: u32, i: usize, iters: u32) -> u32 {
    let len = src.len() as u32;
    let mut x = src[i] ^ seed;
    for _ in 0..iters {
        x = x.wrapping_mul(1664525).wrapping_add(1013904223);
        x ^= x >> 13;
        x = x.wrapping_mul(0x5bd1e995);
        x ^= src[(x % len) as usize];
    }
    x
}

/// xorshift for seeds and sample positions (no rand dependency).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// `0000:01:00.0` → the (vendor, device) ids sysfs gives for it.
fn pci_ids(pci: &str) -> Option<(u32, u32)> {
    let r = |f: &str| {
        std::fs::read_to_string(format!("/sys/bus/pci/devices/{pci}/{f}")).ok().and_then(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok())
    };
    Some((r("vendor")?, r("device")?))
}

fn main() {
    let args = Args::parse();
    match run(&args) {
        Ok(true) => std::process::exit(0),
        Ok(false) => std::process::exit(2),
        Err(e) => {
            println!("{}", serde_json::json!({"event": "error", "message": format!("{e:#}")}));
            std::process::exit(1)
        }
    }
}

fn run(args: &Args) -> Result<bool> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::VULKAN, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN));
    if args.list {
        for a in &adapters {
            let i = a.get_info();
            println!("{:04x}:{:04x} {:?} {}", i.vendor, i.device, i.device_type, i.name);
        }
        return Ok(true);
    }
    let want = args.pci.as_deref().and_then(pci_ids);
    let adapter = adapters
        .into_iter()
        .filter(|a| a.get_info().device_type != wgpu::DeviceType::Cpu)
        .find(|a| {
            let i = a.get_info();
            match want {
                Some((v, d)) => i.vendor == v && i.device == d,
                None => i.device_type == wgpu::DeviceType::DiscreteGpu,
            }
        })
        .context("no matching Vulkan GPU (is its Vulkan driver installed?)")?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("ug-stress"),
        required_limits: wgpu::Limits { max_storage_buffer_binding_size: 1 << 30, max_buffer_size: 1 << 30, ..wgpu::Limits::default() },
        ..Default::default()
    }))
    .context("opening the GPU")?;
    let lost = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let lost = std::sync::Arc::clone(&lost);
        device.set_device_lost_callback(move |_, _| lost.store(true, std::sync::atomic::Ordering::SeqCst));
    }

    let len = (args.megabytes.max(4) as usize * 1024 * 1024 / 4) as u32;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let src: Vec<u32> = (0..len).map(|_| rng.next() as u32).collect();
    let bytes = (len as u64) * 4;
    use wgpu::util::DeviceExt;
    let src_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck_cast(&src),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let dst_buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
    let read_buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let params = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: src_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: dst_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
        ],
    });
    let groups = len.div_ceil(256);
    let (gx, gy) = if groups > 65535 { (65535, groups.div_ceil(65535)) } else { (groups, 1) };

    println!("{}", serde_json::json!({"event": "start", "gpu": info.name, "seconds": args.seconds, "megabytes": args.megabytes}));
    let start = Instant::now();
    let (mut rounds, mut errors, mut checked) = (0u64, 0u64, 0u64);
    let mut iters: u32 = 8;
    let mut last_report = Instant::now();
    while start.elapsed() < Duration::from_secs(args.seconds) {
        let seed = rng.next() as u32;
        let round_start = Instant::now();
        queue.write_buffer(&params, 0, bytemuck_cast(&[seed, len, iters, 0]));
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(gx, gy, 1);
        }
        enc.copy_buffer_to_buffer(&dst_buf, 0, &read_buf, 0, bytes);
        queue.submit([enc.finish()]);
        let slice = read_buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if lost.load(std::sync::atomic::Ordering::SeqCst) {
            println!("{}", serde_json::json!({"event": "done", "stable": false, "reason": "the driver reset the GPU (device lost)", "rounds": rounds}));
            return Ok(false);
        }
        {
            let data = slice.get_mapped_range().context("reading the results back")?;
            let out: &[u32] = bytemuck_from(&data);
            for _ in 0..512 {
                let i = (rng.next() % len as u64) as usize;
                checked += 1;
                let expect = reference(&src, if args.inject_fault { seed ^ 1 } else { seed }, i, iters);
                if out[i] != expect {
                    errors += 1;
                }
            }
        }
        read_buf.unmap();
        rounds += 1;
        // Grow or shrink the work so a round lasts about ROUND.
        let took = round_start.elapsed();
        if took < ROUND / 2 && iters < 4096 {
            iters *= 2;
        } else if took > ROUND * 2 && iters > 1 {
            iters /= 2;
        }
        if errors > 0 {
            println!("{}", serde_json::json!({"event": "done", "stable": false, "reason": format!("{errors} wrong results out of {checked} checked"), "rounds": rounds}));
            return Ok(false);
        }
        if last_report.elapsed() >= Duration::from_secs(1) {
            last_report = Instant::now();
            println!("{}", serde_json::json!({"event": "progress", "elapsed_s": start.elapsed().as_secs(), "rounds": rounds, "checked": checked, "iters": iters}));
        }
    }
    if rounds == 0 {
        bail!("not a single round completed");
    }
    println!("{}", serde_json::json!({"event": "done", "stable": true, "rounds": rounds, "checked": checked}));
    Ok(true)
}

fn bytemuck_cast(v: &[u32]) -> &[u8] {
    // SAFETY: u32 has no padding and any byte pattern is a valid u8.
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

fn bytemuck_from(b: &[u8]) -> &[u32] {
    // SAFETY: the mapped range is a whole number of u32s and is 4-byte aligned
    // (buffer mappings are at least 8-byte aligned).
    unsafe { std::slice::from_raw_parts(b.as_ptr() as *const u32, b.len() / 4) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cpu_reference_is_deterministic_and_seed_dependent() {
        let src: Vec<u32> = (0..1024u32).map(|i| i.wrapping_mul(2654435761)).collect();
        assert_eq!(reference(&src, 7, 3, 64), reference(&src, 7, 3, 64));
        assert_ne!(reference(&src, 7, 3, 64), reference(&src, 8, 3, 64));
    }
}

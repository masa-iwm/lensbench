//! wgpu の装置と Huygens の和（ネイティブ）

use crate::{Error, Reference, Sources};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use wgpu::*;

/// Huygens の和の compute shader。1 つの呼び出しが 1 つの像の点で、光源 [j0, j1) を足して out に加える。
/// src は光源ごとに 2 個: (dir, 振幅), (位相, 0, 0, 0)。pts は (d, 有効なら 1)。
/// P.n = (光源の数, 点の数, j0, j1)、P.g = (R, R², kσ, kn)
const SHADER: &str = r#"
struct Params { n: vec4<u32>, g: vec4<f32> }
@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> pts: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> out: array<vec2<f32>>;
const TAU: f32 = 6.2831853;
const INV_TAU: f32 = 0.15915494;
const WG: u32 = 64u;

fn index(gid: vec3<u32>, nw: vec3<u32>) -> u32 {
    return gid.y * nw.x * WG + gid.x;
}

fn cis(ph: f32) -> vec2<f32> {
    let t = ph - TAU * round(ph * INV_TAU);
    return vec2<f32>(cos(t), sin(t));
}

@compute @workgroup_size(64)
fn sphere(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>) {
    let i = index(gid, nw);
    if (i >= P.n.y) { return; }
    let p = pts[i];
    if (p.w == 0.0) { return; }
    let d = p.xyz;
    let dd = dot(d, d);
    let rr = P.g.x;
    let rr2 = P.g.y;
    var acc = out[i];
    var part = vec2<f32>(0.0, 0.0);
    for (var j = P.n.z; j < P.n.w; j = j + 1u) {
        let s0 = src[2u * j];
        let ph0 = src[2u * j + 1u].x;
        let ad = dot(s0.xyz, d);
        let x = 2.0 * ad + dd;
        let rho = sqrt(rr2 + x);
        let delta = x / (rho + rr);
        let cosx = abs(rr2 + ad) / (rho * rr);
        let amp = s0.w * rr / rho * 0.5 * (1.0 + cosx);
        part = part + amp * cis(ph0 - P.g.z * delta);
        if ((j & 255u) == 255u) { acc = acc + part; part = vec2<f32>(0.0, 0.0); }
    }
    out[i] = acc + part;
}

@compute @workgroup_size(64)
fn plane(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nw: vec3<u32>) {
    let i = index(gid, nw);
    if (i >= P.n.y) { return; }
    let p = pts[i];
    if (p.w == 0.0) { return; }
    let d = p.xyz;
    var acc = out[i];
    var part = vec2<f32>(0.0, 0.0);
    for (var j = P.n.z; j < P.n.w; j = j + 1u) {
        let s0 = src[2u * j];
        let ph0 = src[2u * j + 1u].x;
        part = part + s0.w * cis(ph0 - P.g.w * dot(s0.xyz, d));
        if ((j & 255u) == 255u) { acc = acc + part; part = vec2<f32>(0.0, 0.0); }
    }
    out[i] = acc + part;
}
"#;

/// 1 回の投入で足す (像の点, 光源) の組の数の目安。RTX 3060 で約 4 ms、Intel UHD 630 で約 56 ms（2 s より十分短い）
const PAIRS_PER_SUBMIT: usize = 1 << 28;
/// 1 次元の投入の作業グループの数の上限（WebGPU の既定の制限）
const MAX_GROUPS: u32 = 65535;
const WG: usize = 64;

/// GPU の装置
pub struct Gpu {
    device: Device,
    queue: Queue,
    sphere: ComputePipeline,
    plane: ComputePipeline,
    name: String,
    software: bool,
    /// 既定（auto）で使ってよいか（CPU で動くアダプタと、macOS 以外の内蔵 GPU は使わない）
    auto: bool,
    /// 装置を失った・捕まえていない誤りが出た（以後は使わない）
    broken: Arc<AtomicBool>,
}

static ADAPTER: OnceLock<Option<Adapter>> = OnceLock::new();
static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

/// 使う装置。forced が false（auto）なら [`auto_ok`] でないアダプタでは装置を作らない
/// （内蔵 GPU のドライバの中で落ちることがあるので、使わないアダプタには触らない）
pub(crate) fn get(forced: bool) -> Option<&'static Gpu> {
    let adapter = ADAPTER.get_or_init(pick_adapter).as_ref()?;
    if !forced && !auto_ok(&adapter.get_info()) {
        return None;
    }
    GPU.get_or_init(|| init(adapter))
        .as_ref()
        .filter(|g| !g.broken.load(Ordering::Relaxed))
}

/// 既定（auto）で使ってよいアダプタか。CPU で動くもの（WARP・llvmpipe）と、macOS 以外の内蔵 GPU は使わない
/// （Intel UHD 630 は 8 コアの CPU の既定の展開と同じかそれより遅い。Apple の GPU は内蔵の種別でも速い）
fn auto_ok(info: &AdapterInfo) -> bool {
    match info.device_type {
        DeviceType::Cpu => false,
        DeviceType::IntegratedGpu => info.backend == Backend::Metal,
        _ => true,
    }
}

fn pick_adapter() -> Option<Adapter> {
    let inst = Instance::new(InstanceDescriptor::new_without_display_handle().with_env());
    let adapter = pollster::block_on(inst.request_adapter(&RequestAdapterOptions {
        // WGPU_POWER_PREF=low で内蔵 GPU を選べる（測るとき用、既定は外付け）
        power_preference: PowerPreference::from_env().unwrap_or(PowerPreference::HighPerformance),
        force_fallback_adapter: false,
        compatible_surface: None,
        ..Default::default()
    }))
    .ok()?;
    let info = adapter.get_info();
    // Windows の内蔵 GPU は、バックエンドを指定していなければ Vulkan より DX12 で（Intel UHD 630 の Vulkan の
    // ドライバは装置を作る所でアクセス違反で落ちた。DX12 では動いた）
    if cfg!(windows)
        && info.backend == Backend::Vulkan
        && info.device_type == DeviceType::IntegratedGpu
        && std::env::var_os("WGPU_BACKEND").is_none()
    {
        let dx = pollster::block_on(inst.enumerate_adapters(Backends::DX12));
        if let Some(a) = dx.into_iter().find(|a| {
            let i = a.get_info();
            i.vendor == info.vendor && i.device == info.device
        }) {
            return Some(a);
        }
    }
    Some(adapter)
}

fn init(adapter: &Adapter) -> Option<Gpu> {
    let info = adapter.get_info();
    let al = adapter.limits();
    let limits = Limits {
        max_storage_buffer_binding_size: al.max_storage_buffer_binding_size,
        max_buffer_size: al.max_buffer_size,
        ..Limits::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
        label: Some("lensbench"),
        required_features: Features::empty(),
        required_limits: limits,
        ..Default::default()
    }))
    .ok()?;
    let broken = Arc::new(AtomicBool::new(false));
    {
        let b = broken.clone();
        device.on_uncaptured_error(Arc::new(move |e| {
            eprintln!("lensbench: GPU error (falling back to the CPU): {e}");
            b.store(true, Ordering::Relaxed);
        }));
        let b = broken.clone();
        device.set_device_lost_callback(move |_, msg| {
            if !msg.is_empty() {
                eprintln!("lensbench: GPU device lost (falling back to the CPU): {msg}");
            }
            b.store(true, Ordering::Relaxed);
        });
    }
    let scope = device.push_error_scope(ErrorFilter::Validation);
    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("huygens"),
        source: ShaderSource::Wgsl(SHADER.into()),
    });
    let pipe = |entry: &str| {
        device.create_compute_pipeline(&ComputePipelineDescriptor {
            label: Some(entry),
            layout: None,
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        })
    };
    let (sphere, plane) = (pipe("sphere"), pipe("plane"));
    if let Some(e) = pollster::block_on(scope.pop()) {
        eprintln!("lensbench: GPU pipeline error (using the CPU): {e}");
        return None;
    }
    Some(Gpu {
        device,
        queue,
        sphere,
        plane,
        name: format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type),
        software: info.device_type == DeviceType::Cpu,
        auto: auto_ok(&info),
        broken,
    })
}

/// 位相 [rad] を [−π, π) に（倍精度で）
fn wrap(ph: f64) -> f32 {
    use std::f64::consts::TAU;
    (ph - TAU * (ph / TAU).round()) as f32
}

impl Gpu {
    /// アダプタの名前（バックエンド・種別）
    pub fn name(&self) -> &str {
        &self.name
    }

    /// CPU で動くアダプタ（WARP・llvmpipe）か
    pub fn software(&self) -> bool {
        self.software
    }

    /// 既定（auto）で使ってよいか（CPU で動くアダプタと、macOS 以外の内蔵 GPU は false）
    pub fn for_auto(&self) -> bool {
        self.auto
    }

    /// 像の点 pts（球: d = P − C、平面: d = P − P₀。有限でない点は NaN を返す）ごとの |Σ amp exp(i 位相)|²。
    /// cancel は投入の合間に確かめる
    pub fn huygens(
        &self,
        r: Reference,
        s: Sources<'_>,
        pts: &[[f64; 3]],
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<f64>, Error> {
        let ns = s.amp.len();
        if s.dir.len() != ns || s.phase.len() != ns {
            return Err(Error::Gpu("source arrays of different lengths".into()));
        }
        let npts = pts.len();
        if ns == 0 || npts == 0 {
            return Ok(pts
                .iter()
                .map(|p| if ok(p) { 0.0 } else { f64::NAN })
                .collect());
        }
        let scope = self.device.push_error_scope(ErrorFilter::Validation);
        let oom = self.device.push_error_scope(ErrorFilter::OutOfMemory);
        let res = self.run(r, s, pts, cancel);
        let e_oom = pollster::block_on(oom.pop());
        let e_val = pollster::block_on(scope.pop());
        if let Some(e) = e_oom.or(e_val) {
            return Err(Error::Gpu(e.to_string()));
        }
        res
    }

    fn run(
        &self,
        r: Reference,
        s: Sources<'_>,
        pts: &[[f64; 3]],
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<f64>, Error> {
        let (dev, queue) = (&self.device, &self.queue);
        let ns = s.amp.len();
        let npts = pts.len();
        // LENSBENCH_GPU_TRACE があれば段ごとの時間を標準エラーに出す（休んでいた GPU の 1 回目の遅さの切り分け用）
        let trace = std::env::var_os("LENSBENCH_GPU_TRACE").is_some();
        let t0 = std::time::Instant::now();
        let mut marks: Vec<(&str, f64)> = Vec::new();
        let mut mark = |what: &'static str| {
            if trace {
                marks.push((what, t0.elapsed().as_secs_f64()));
            }
        };
        let mut src = Vec::with_capacity(ns * 8);
        for j in 0..ns {
            let d = s.dir[j];
            src.extend_from_slice(&[
                d[0] as f32,
                d[1] as f32,
                d[2] as f32,
                s.amp[j] as f32,
                wrap(s.phase[j]),
                0.0,
                0.0,
                0.0,
            ]);
        }
        let pv: Vec<f32> = pts
            .iter()
            .flat_map(|p| {
                if ok(p) {
                    [p[0] as f32, p[1] as f32, p[2] as f32, 1.0]
                } else {
                    [0.0; 4]
                }
            })
            .collect();
        let storage = |label: &str, bytes: &[u8]| {
            let b = dev.create_buffer(&BufferDescriptor {
                label: Some(label),
                size: bytes.len().max(16) as u64,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&b, 0, bytes);
            b
        };
        let src_b = storage("sources", bytemuck::cast_slice(&src));
        let pts_b = storage("points", bytemuck::cast_slice(&pv));
        let out_size = npts as u64 * 8;
        let out = dev.create_buffer(&BufferDescriptor {
            label: Some("sums"),
            size: out_size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let read = dev.create_buffer(&BufferDescriptor {
            label: Some("readback"),
            size: out_size,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uni = dev.create_buffer(&BufferDescriptor {
            label: Some("params"),
            size: 32,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (pipe, g) = match r {
            Reference::Sphere { radius, ksg } => (
                &self.sphere,
                [radius as f32, (radius * radius) as f32, ksg as f32, 0.0],
            ),
            Reference::Plane { kn } => (&self.plane, [0.0, 0.0, 0.0, kn as f32]),
        };
        let bg = dev.create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipe.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: uni.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: src_b.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: pts_b.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: out.as_entire_binding(),
                },
            ],
        });
        let groups = npts.div_ceil(WG);
        let (gx, gy) = if groups <= MAX_GROUPS as usize {
            (groups as u32, 1)
        } else {
            (MAX_GROUPS, groups.div_ceil(MAX_GROUPS as usize) as u32)
        };
        let wait = || {
            dev.poll(PollType::wait_indefinitely())
                .map(|_| ())
                .map_err(|e| Error::Gpu(e.to_string()))
        };
        let mut enc = dev.create_command_encoder(&CommandEncoderDescriptor::default());
        enc.clear_buffer(&out, 0, None);
        queue.submit([enc.finish()]);
        if trace {
            wait()?;
        }
        mark("buffers");
        let step = (PAIRS_PER_SUBMIT / npts).max(64);
        let mut j0 = 0;
        while j0 < ns {
            if cancel() {
                wait()?;
                return Err(Error::Cancelled);
            }
            let j1 = (j0 + step).min(ns);
            let mut par = [0u8; 32];
            par[..16].copy_from_slice(bytemuck::cast_slice(&[
                ns as u32,
                npts as u32,
                j0 as u32,
                j1 as u32,
            ]));
            par[16..].copy_from_slice(bytemuck::cast_slice(&g));
            queue.write_buffer(&uni, 0, &par);
            let mut enc = dev.create_command_encoder(&CommandEncoderDescriptor::default());
            {
                let mut pass = enc.begin_compute_pass(&ComputePassDescriptor::default());
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, &bg, &[]);
                pass.dispatch_workgroups(gx, gy, 1);
            }
            queue.submit([enc.finish()]);
            // 1 回ずつ終わるのを待つ（次の write_buffer の前に。Windows の 2 s の制限は投入ごと）
            wait()?;
            mark("submit");
            j0 = j1;
        }
        let mut enc = dev.create_command_encoder(&CommandEncoderDescriptor::default());
        enc.copy_buffer_to_buffer(&out, 0, &read, 0, out_size);
        queue.submit([enc.finish()]);
        let slice = read.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        wait()?;
        match rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(Error::Gpu(e.to_string())),
            Err(e) => return Err(Error::Gpu(e.to_string())),
        }
        let v: Vec<f64> = {
            let m = slice
                .get_mapped_range()
                .map_err(|e| Error::Gpu(e.to_string()))?;
            bytemuck::cast_slice::<u8, f32>(&m)
                .as_chunks::<2>()
                .0
                .iter()
                .zip(pts)
                .map(|(c, p)| {
                    if ok(p) {
                        (c[0] as f64).powi(2) + (c[1] as f64).powi(2)
                    } else {
                        f64::NAN
                    }
                })
                .collect()
        };
        read.unmap();
        mark("readback");
        if trace {
            let mut prev = 0.0;
            let parts: Vec<String> = marks
                .iter()
                .map(|&(w, t)| {
                    let d = t - prev;
                    prev = t;
                    format!("{w} {:.1}", d * 1e3)
                })
                .collect();
            eprintln!(
                "gpu trace: {ns} sources x {npts} points, {:.1} ms: {} (ms)",
                t0.elapsed().as_secs_f64() * 1e3,
                parts.join(", ")
            );
        }
        Ok(v)
    }
}

fn ok(p: &[f64; 3]) -> bool {
    p.iter().all(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 倍精度の素直な和
    fn cpu(r: Reference, s: Sources<'_>, pts: &[[f64; 3]]) -> Vec<f64> {
        pts.iter()
            .map(|d| {
                let (mut re, mut im) = (0.0, 0.0);
                for j in 0..s.amp.len() {
                    let a = s.dir[j];
                    let (amp, ph) = match r {
                        Reference::Sphere { radius: rr, ksg } => {
                            let q = [-a[0], -a[1], -a[2]];
                            let v = [d[0] - q[0], d[1] - q[1], d[2] - q[2]];
                            let rho = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                            let cosx =
                                ((v[0] * a[0] + v[1] * a[1] + v[2] * a[2]) / (rho * rr)).abs();
                            (
                                s.amp[j] * rr / rho * 0.5 * (1.0 + cosx),
                                s.phase[j] - ksg * (rho - rr),
                            )
                        }
                        Reference::Plane { kn } => (
                            s.amp[j],
                            s.phase[j] - kn * (a[0] * d[0] + a[1] * d[1] + a[2] * d[2]),
                        ),
                    };
                    re += amp * ph.cos();
                    im += amp * ph.sin();
                }
                re * re + im * im
            })
            .collect()
    }

    /// 合成の光源（F/5 ほどの球、半径 50 mm、波長 0.5 µm）で倍精度の和と比べる。GPU が無ければ飛ばす
    #[test]
    fn matches_double_precision() {
        let Some(g) = crate::gpu(true) else {
            eprintln!("no GPU adapter; skipped");
            return;
        };
        let rr = 50.0;
        let k = 2.0 * std::f64::consts::PI / 0.5e-3;
        let n = 40;
        let (mut dir, mut dk, mut amp, mut phase) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for iy in 0..n {
            for ix in 0..n {
                let (u, v) = (
                    (ix as f64 + 0.5) / n as f64 * 2.0 - 1.0,
                    (iy as f64 + 0.5) / n as f64 * 2.0 - 1.0,
                );
                if u * u + v * v > 1.0 {
                    continue;
                }
                let na = 0.1;
                let (l, m) = (u * na, v * na);
                let nz = (1.0 - l * l - m * m).sqrt();
                dir.push([l * rr, m * rr, nz * rr]);
                dk.push([l, m, nz - 1.0]);
                amp.push(1.0 / (n * n) as f64);
                // 球面収差 2 波＋大きな定数
                let r2 = u * u + v * v;
                phase.push(2.0 * std::f64::consts::PI * (2.0 * r2 * r2 + 1234.0));
            }
        }
        let sphere = Sources {
            dir: &dir,
            amp: &amp,
            phase: &phase,
        };
        let plane = Sources { dir: &dk, ..sphere };
        let px = 2.5e-3;
        let pts: Vec<[f64; 3]> = (0..32)
            .flat_map(|j| {
                (0..32).map(move |i| [(i as f64 - 16.0) * px, (j as f64 - 16.0) * px, 0.0])
            })
            .chain([[f64::NAN, 0.0, 0.0]])
            .collect();
        let peak = amp.iter().sum::<f64>().powi(2);
        for (r, s) in [
            (Reference::Sphere { radius: rr, ksg: k }, sphere),
            (Reference::Plane { kn: k }, plane),
        ] {
            let got = g.huygens(r, s, &pts, &|| false).expect("GPU sum");
            let want = cpu(r, s, &pts);
            assert!(got[pts.len() - 1].is_nan());
            let d = got
                .iter()
                .zip(&want)
                .take(pts.len() - 1)
                .map(|(a, b)| (a - b).abs() / peak)
                .fold(0.0, f64::max);
            assert!(d < 2e-6, "{r:?}: {d:e} ({})", g.name());
        }
        assert_eq!(
            g.huygens(Reference::Plane { kn: k }, plane, &pts, &|| true),
            Err(Error::Cancelled)
        );
    }
}

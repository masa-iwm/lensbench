//! serve の TCP 以外の受け渡し: 名前付きパイプ（Windows）・Unix ドメインソケット（Linux など）と、
//! 配列を置く共有メモリ。
//!
//! - `--pipe NAME`: Windows は `\\.\pipe\NAME` の名前付きパイプ（同じ PC からだけ。リモートの接続は断る）、
//!   ほかは NAME のパスの Unix ドメインソケット（作った利用者だけが読み書きできる 0600）。書式は TCP と同じ
//! - 共有メモリ: 要求のヘッダの `"shm": {"name": 名前}` と、bin の `"offset"`（バイト）で、配列をストリームでなく
//!   共有メモリから読む。応答の配列も、収まれば同じ共有メモリ（`"out"` のバイト位置から）に書く。
//!   名前は Python の `multiprocessing.shared_memory.SharedMemory` の name（Windows は名前付きのファイル
//!   マッピング、Linux は /dev/shm のファイル）

use std::io::{Read, Write};

/// 共有メモリ（名前で開いた区画）
pub(crate) struct Shm {
    #[cfg(target_os = "linux")]
    f: std::fs::File,
    #[cfg(windows)]
    view: win::View,
    size: u64,
}

/// 共有メモリの名前（英数字と _ . - だけ。パスにならないように）
fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 200
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        || name.starts_with('.')
    {
        return Err(format!("共有メモリの名前 {name:?}（英数字と _ . - だけ）"));
    }
    Ok(())
}

impl Shm {
    /// 名前で開く（作るのはクライアント）
    pub fn open(name: &str) -> Result<Shm, String> {
        check_name(name)?;
        #[cfg(target_os = "linux")]
        {
            let p = std::path::Path::new("/dev/shm").join(name);
            let f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&p)
                .map_err(|e| format!("共有メモリ {name}: {e}"))?;
            let size = f.metadata().map_err(|e| e.to_string())?.len();
            Ok(Shm { f, size })
        }
        #[cfg(windows)]
        {
            let view = win::View::open(name)?;
            let size = view.size as u64;
            Ok(Shm { view, size })
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            Err("共有メモリは Windows・Linux だけ".into())
        }
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    fn check(&self, off: u64, n: usize) -> Result<(), String> {
        match off.checked_add(n as u64) {
            Some(e) if e <= self.size => Ok(()),
            _ => Err(format!(
                "共有メモリの範囲の外（位置 {off}、{n} バイト、大きさ {}）",
                self.size
            )),
        }
    }

    /// off からのバイト列を読む
    pub fn read(&self, off: u64, buf: &mut [u8]) -> Result<(), String> {
        self.check(off, buf.len())?;
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::FileExt;
            self.f
                .read_exact_at(buf, off)
                .map_err(|e| format!("共有メモリ: {e}"))
        }
        #[cfg(windows)]
        {
            self.view.read(off as usize, buf);
            Ok(())
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            let _ = buf;
            Err("共有メモリは Windows・Linux だけ".into())
        }
    }

    /// off からバイト列を書く
    pub fn write(&self, off: u64, data: &[u8]) -> Result<(), String> {
        self.check(off, data.len())?;
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::FileExt;
            self.f
                .write_all_at(data, off)
                .map_err(|e| format!("共有メモリ: {e}"))
        }
        #[cfg(windows)]
        {
            self.view.write(off as usize, data);
            Ok(())
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            let _ = data;
            Err("共有メモリは Windows・Linux だけ".into())
        }
    }
}

/// 応答を書いた後、相手が読み終える前に閉じても失われないようにする（shutdown で止める前）
pub(crate) trait HardFlush {
    fn hard_flush(&self) {}
}

impl HardFlush for std::net::TcpStream {}

#[cfg(unix)]
impl HardFlush for std::os::unix::net::UnixStream {}

#[cfg(windows)]
impl HardFlush for std::fs::File {
    fn hard_flush(&self) {
        // 名前付きパイプでは FlushFileBuffers が、相手が読み終えるまで待つ
        let _ = self.sync_all();
    }
}

/// パイプの名前から、待ち受けるパス（Windows は \\.\pipe\ を前に付ける）
pub(crate) fn pipe_path(name: &str) -> String {
    #[cfg(windows)]
    {
        if name.starts_with(r"\\.\pipe\") {
            name.to_string()
        } else {
            format!(r"\\.\pipe\{name}")
        }
    }
    #[cfg(not(windows))]
    {
        name.to_string()
    }
}

/// パイプで待ち受ける。待ち受けを始めたら ready を呼び、接続ごとにスレッドで conn を呼ぶ
#[cfg(unix)]
pub(crate) fn listen_pipe(
    path: &str,
    ready: impl FnOnce() -> Result<(), String>,
    conn: impl Fn(Box<dyn Read + Send>, Box<dyn Write + Send>, Box<dyn HardFlush + Send>)
        + Send
        + Sync
        + 'static,
) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    let bind = || UnixListener::bind(path);
    let l = match bind() {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            // 前に止まったサーバーの残りのソケットなら消して作り直す（動いているサーバーや、ソケットでない
            // ファイルがあれば断る）
            use std::os::unix::fs::FileTypeExt;
            let sock = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_socket());
            if !sock {
                return Err(format!("--pipe {path} はソケットでないファイルがある"));
            }
            if UnixStream::connect(path).is_ok() {
                return Err(format!("--pipe {path} は別のサーバーが使っている"));
            }
            lensbench_core::vfs::remove_file(path).map_err(|e| format!("--pipe {path}: {e}"))?;
            bind().map_err(|e| format!("--pipe {path}: {e}"))?
        }
        Err(e) => return Err(format!("--pipe {path}: {e}")),
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("--pipe {path}: {e}"))?;
    ready()?;
    let conn = std::sync::Arc::new(conn);
    for c in l.incoming() {
        match c.and_then(|c| Ok((c.try_clone()?, c.try_clone()?, c))) {
            Ok((r, w, f)) => {
                let conn = conn.clone();
                std::thread::spawn(move || conn(Box::new(r), Box::new(w), Box::new(f)));
            }
            Err(e) => eprintln!("lensbench serve: {e}"),
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn listen_pipe(
    path: &str,
    ready: impl FnOnce() -> Result<(), String>,
    conn: impl Fn(Box<dyn Read + Send>, Box<dyn Write + Send>, Box<dyn HardFlush + Send>)
        + Send
        + Sync
        + 'static,
) -> Result<(), String> {
    let conn = std::sync::Arc::new(conn);
    let mut ready = Some(ready);
    loop {
        let f = win::pipe_instance(path, ready.is_some())?;
        if let Some(r) = ready.take() {
            r()?;
        }
        let f = match win::connect(f) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("lensbench serve: {e}");
                continue;
            }
        };
        match (f.try_clone(), f.try_clone()) {
            (Ok(r), Ok(w)) => {
                let conn = conn.clone();
                std::thread::spawn(move || conn(Box::new(r), Box::new(w), Box::new(f)));
            }
            (Err(e), _) | (_, Err(e)) => eprintln!("lensbench serve: {e}"),
        }
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn listen_pipe(
    path: &str,
    _ready: impl FnOnce() -> Result<(), String>,
    _conn: impl Fn(Box<dyn Read + Send>, Box<dyn Write + Send>, Box<dyn HardFlush + Send>)
        + Send
        + Sync
        + 'static,
) -> Result<(), String> {
    Err(format!("--pipe {path} はこの OS では使えない"))
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::fs::File;
    use std::os::windows::io::{FromRawHandle, RawHandle};

    type Handle = *mut c_void;

    const PIPE_ACCESS_DUPLEX: u32 = 0x3;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
    const PIPE_TYPE_BYTE: u32 = 0;
    const PIPE_READMODE_BYTE: u32 = 0;
    const PIPE_WAIT: u32 = 0;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x8;
    const PIPE_UNLIMITED_INSTANCES: u32 = 255;
    const ERROR_PIPE_CONNECTED: i32 = 535;
    const FILE_MAP_WRITE: u32 = 0x2;
    const FILE_MAP_READ: u32 = 0x4;

    #[repr(C)]
    struct MemoryBasicInformation {
        base_address: *mut c_void,
        allocation_base: *mut c_void,
        allocation_protect: u32,
        #[cfg(target_pointer_width = "64")]
        partition_id: u16,
        region_size: usize,
        state: u32,
        protect: u32,
        typ: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: u32,
            pipe_mode: u32,
            max_instances: u32,
            out_buffer: u32,
            in_buffer: u32,
            timeout: u32,
            security: *mut c_void,
        ) -> Handle;
        fn ConnectNamedPipe(h: Handle, overlapped: *mut c_void) -> i32;
        fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> Handle;
        fn MapViewOfFile(h: Handle, access: u32, off_hi: u32, off_lo: u32, n: usize)
            -> *mut c_void;
        fn UnmapViewOfFile(p: *const c_void) -> i32;
        fn VirtualQuery(p: *const c_void, info: *mut MemoryBasicInformation, len: usize) -> usize;
        fn CloseHandle(h: Handle) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// パイプの口を 1 つ作る（最初の口は、同じ名前のパイプが既にあれば断る）
    pub fn pipe_instance(path: &str, first: bool) -> Result<File, String> {
        let name = wide(path);
        let mode = PIPE_ACCESS_DUPLEX
            | if first {
                FILE_FLAG_FIRST_PIPE_INSTANCE
            } else {
                0
            };
        // SAFETY: name は 0 で終わる UTF-16。セキュリティ記述子は既定（作った利用者・管理者が読み書きできる）
        let h = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                1 << 16,
                1 << 16,
                0,
                std::ptr::null_mut(),
            )
        };
        if h.is_null() || h as isize == -1 {
            return Err(format!(
                "--pipe {path} で待ち受けられない: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: h は今作った、ほかで使っていないハンドル（File が閉じる）
        Ok(unsafe { File::from_raw_handle(h as RawHandle) })
    }

    /// 相手がつなぐまで待つ
    pub fn connect(f: File) -> Result<File, String> {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: f はパイプの口のハンドル。同期の待ち（overlapped なし）
        let ok = unsafe { ConnectNamedPipe(f.as_raw_handle() as Handle, std::ptr::null_mut()) };
        if ok == 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() != Some(ERROR_PIPE_CONNECTED) {
                return Err(format!("パイプの接続: {e}"));
            }
        }
        Ok(f)
    }

    /// 名前付きのファイルマッピングの全体を写した区画
    pub struct View {
        h: Handle,
        p: *mut u8,
        pub size: usize,
    }

    impl View {
        pub fn open(name: &str) -> Result<View, String> {
            let w = wide(name);
            // SAFETY: w は 0 で終わる UTF-16
            let h = unsafe { OpenFileMappingW(FILE_MAP_READ | FILE_MAP_WRITE, 0, w.as_ptr()) };
            if h.is_null() {
                return Err(format!(
                    "共有メモリ {name}: {}",
                    std::io::Error::last_os_error()
                ));
            }
            // SAFETY: h は開いたマッピング。0 バイトで全体を写す
            let p = unsafe { MapViewOfFile(h, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, 0) };
            if p.is_null() {
                let e = std::io::Error::last_os_error();
                // SAFETY: h は開いたハンドル
                unsafe { CloseHandle(h) };
                return Err(format!("共有メモリ {name}: {e}"));
            }
            // SAFETY: 写した区画の先頭で問い合わせる
            let mut mbi: MemoryBasicInformation = unsafe { std::mem::zeroed() };
            let n =
                unsafe { VirtualQuery(p, &mut mbi, std::mem::size_of::<MemoryBasicInformation>()) };
            let size = if n == 0 { 0 } else { mbi.region_size };
            Ok(View {
                h,
                p: p as *mut u8,
                size,
            })
        }

        pub fn read(&self, off: usize, buf: &mut [u8]) {
            // SAFETY: 範囲は呼ぶ側（Shm::check）で size の中と確かめた
            unsafe { std::ptr::copy_nonoverlapping(self.p.add(off), buf.as_mut_ptr(), buf.len()) }
        }

        pub fn write(&self, off: usize, data: &[u8]) {
            // SAFETY: 範囲は呼ぶ側（Shm::check）で size の中と確かめた
            unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), self.p.add(off), data.len()) }
        }
    }

    impl Drop for View {
        fn drop(&mut self) {
            // SAFETY: open で写した区画と開いたハンドル
            unsafe {
                UnmapViewOfFile(self.p as *const c_void);
                CloseHandle(self.h);
            }
        }
    }
}

"""lensbench serve の薄いクライアント（標準ライブラリだけで動く。numpy があれば配列は numpy で返す）。

別のプロセスで動く lensbench に系を読み込ませたまま、値を変えて解析を繰り返す::

    from lensbench_client import Client

    with Client.spawn("sys.zmx", glass_dirs=["glass"], threads=1) as lb:
        for t in [1.0, 1.2, 1.4]:
            lb.set({"3.t": t})
            print(t, lb.analyze("spot", field=1)["spot"][0]["rms_centroid"])
        r = lb.trace(px, py, field=2)        # r["x"], r["y"], ... は numpy の配列

TCP の代わりにパイプ（Windows は名前付きパイプ、ほかは Unix ドメインソケット）でつなぐなら spawn(..., pipe=True)、
大きい配列を共有メモリで渡すなら use_shm(バイト数)（numpy が要る）。

プロセスの中で速く呼ぶなら Python バインディング（import lensbench）を使う。書式は docs/serve.md。
"""

from __future__ import annotations

import io
import json
import os
import secrets
import socket
import subprocess
import sys
import time
import warnings
from array import array

try:  # numpy は無くてもよい
    import numpy as _np
except ImportError:  # pragma: no cover
    _np = None

__all__ = ["Client", "RpcError"]

_ITEM = {"<f8": ("d", 8), "<i4": ("i", 4), "|u1": ("B", 1), "|b1": ("B", 1), "<c16": ("d", 16)}
_WIN = sys.platform == "win32"


class RpcError(RuntimeError):
    """サーバーが返したエラー（code は JSON-RPC のエラー番号、kind は計算の誤りの種類
    "invalid"・"not_implemented"・"io"・"cancelled"。それ以外は None）"""

    def __init__(self, code: int, message: str, kind: str | None = None):
        super().__init__(f"{message} (code {code})")
        self.code = code
        self.message = message
        self.kind = kind


def _to_bytes(v):
    """配列を (dtype, shape, bytes) にする"""
    if _np is not None and isinstance(v, _np.ndarray):
        if v.dtype.kind == "b":
            a = _np.ascontiguousarray(v, dtype="|b1")
            return "|b1", list(a.shape), a.tobytes()
        if v.dtype.kind == "c":
            a = _np.ascontiguousarray(v, dtype="<c16")
            return "<c16", list(a.shape), a.tobytes()
        if v.dtype.kind == "f":
            a = _np.ascontiguousarray(v, dtype="<f8")
            return "<f8", list(a.shape), a.tobytes()
        if v.dtype.kind in "iub" and v.dtype.itemsize == 1 and v.dtype.kind != "i":
            a = _np.ascontiguousarray(v, dtype="|u1")
            return "|u1", list(a.shape), a.tobytes()
        a = _np.ascontiguousarray(v, dtype="<i4")
        return "<i4", list(a.shape), a.tobytes()
    if isinstance(v, (bytes, bytearray)):
        return "|u1", [len(v)], bytes(v)
    if isinstance(v, array) and v.typecode in ("i", "l") and v.itemsize == 4:
        a = v
        if sys.byteorder != "little":
            a = array(v.typecode, v)
            a.byteswap()
        return "<i4", [len(v)], a.tobytes()
    a = array("d", v)
    if sys.byteorder != "little":
        a.byteswap()
    return "<f8", [len(a)], a.tobytes()


def _from_bytes(dtype, shape, b):
    if _np is not None:
        return _np.frombuffer(b, dtype=dtype).reshape(shape)
    code, _ = _ITEM[dtype]
    a = array(code)
    a.frombytes(b)
    if sys.byteorder != "little" and code != "B":
        a.byteswap()
    return a


class Client:
    """lensbench serve への 1 本の接続。要求は順に 1 つずつ（スレッド間で共有しない）"""

    def __init__(
        self, host: str = "127.0.0.1", port: int = 7010, timeout: float | None = None, *, pipe: str | None = None
    ):
        """host・port の TCP か、pipe（serve --pipe の名前かパス）につなぐ"""
        self._addr = {"host": host, "port": port, "timeout": timeout, "pipe": pipe}
        self._sock = None
        self._f = None
        if pipe is not None and _WIN:
            self._f = _open_win_pipe(pipe, 5.0 if timeout is None else timeout)
            self._r = io.BufferedReader(self._f, buffer_size=1 << 16)
        else:
            if pipe is not None:
                self._sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                self._sock.settimeout(timeout)
                self._sock.connect(os.fspath(pipe))
            else:
                self._sock = socket.create_connection((host, port), timeout=timeout)
                self._sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            self._r = self._sock.makefile("rb", buffering=1 << 16)
        self._id = 0
        self._proc: subprocess.Popen | None = None
        self._pipe_path: str | None = None
        self._shm = None
        #: 直前の要求のサーバー内の処理時間 [ms]（通信を除く）
        self.elapsed_ms: float | None = None

    # ---- 起動 ----

    @classmethod
    def spawn(
        cls,
        file: str | os.PathLike | None = None,
        *,
        exe: str | os.PathLike | None = None,
        threads: int | None = None,
        glass_dirs=None,
        lens_catalogs=None,
        cfg: int | None = None,
        state: str | None = None,
        beam_path: str | None = None,
        args=(),
        startup_timeout: float = 30.0,
        pipe: bool | str = False,
        glass_dir=None,
        lens_catalog=None,
    ) -> "Client":
        """lensbench serve を空いているポートで起動してつなぐ。閉じる（close・with を抜ける）とサーバーも止める。

        exe を省くと環境変数 LENSBENCH_EXE、無ければ PATH の lensbench を使う。pipe=True（か名前・パス）なら
        TCP の代わりにパイプ（Windows は名前付きパイプ、ほかは一時フォルダの Unix ドメインソケット）でつなぐ。
        glass_dir・lens_catalog は glass_dirs・lens_catalogs の旧名。
        """
        glass_dirs = _renamed(glass_dirs, glass_dir, "glass_dirs", "glass_dir")
        lens_catalogs = _renamed(lens_catalogs, lens_catalog, "lens_catalogs", "lens_catalog")
        exe = os.fspath(exe or os.environ.get("LENSBENCH_EXE") or "lensbench")
        cmd = [exe]
        if threads is not None:
            cmd += ["--threads", str(threads)]
        pipe_name = None
        if pipe:
            pipe_name = pipe if isinstance(pipe, str) else f"lensbench-{os.getpid()}-{secrets.token_hex(4)}"
            if not _WIN and os.sep not in pipe_name:
                import tempfile

                pipe_name = os.path.join(tempfile.gettempdir(), pipe_name + ".sock")
            cmd += ["serve", "--pipe", pipe_name]
        else:
            cmd += ["serve", "--port", "0"]
        if file is not None:
            cmd.append(os.fspath(file))
        for g in _paths(glass_dirs):
            cmd += ["--glass-dir", g]
        for g in _paths(lens_catalogs):
            cmd += ["--lens-catalog", g]
        if cfg is not None:
            cmd += ["--cfg", str(cfg)]
        if state is not None:
            cmd += ["--state", state]
        if beam_path is not None:
            cmd += ["--beam-path", beam_path]
        cmd += list(args)
        p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=None)
        line = _readline_timeout(p, startup_timeout)
        if not line:
            p.wait(timeout=5)
            raise RuntimeError(f"lensbench serve が起動しない（終了コード {p.returncode}）: {' '.join(cmd)}")
        head = json.loads(line)
        c = cls(pipe=head["pipe"]) if pipe_name else cls("127.0.0.1", head["port"])
        c._proc = p
        if pipe_name and not _WIN:
            c._pipe_path = head["pipe"]
        return c

    # ---- 共有メモリ ----

    def use_shm(self, size: int):
        """大きさ size [バイト] の共有メモリを作り、以後の配列の受け渡しに使う（numpy が要る）。要求の配列が
        収まれば共有メモリに置き、応答の配列も収まればサーバーがそこに書く（収まらなければ今までどおり送る）。
        use_shm(0) でやめる。同じ PC のサーバー（--allow-remote なし）だけ"""
        from multiprocessing import shared_memory

        if _np is None:
            raise RuntimeError("共有メモリには numpy が要る")
        self._close_shm()
        if size > 0:
            self._shm = shared_memory.SharedMemory(create=True, size=int(size))

    def _close_shm(self):
        if self._shm is not None:
            self._shm.close()
            try:
                self._shm.unlink()
            except FileNotFoundError:  # pragma: no cover
                pass
            self._shm = None

    # ---- 低水準 ----

    def call(self, method: str, params: dict | None = None, arrays: dict | None = None):
        """要求を送り (result, 配列の dict) を返す。arrays は {名前: 配列}（バイナリで送る）"""
        self._id += 1
        head = {"jsonrpc": "2.0", "id": self._id, "method": method, "params": params or {}}
        chunks = []
        if arrays:
            bins = []
            items = [(k, _to_bytes(v)) for k, v in arrays.items()]
            shm = self._shm
            if shm is not None and sum(len(b) + 64 for _, (_, _, b) in items) > shm.size:
                shm = None
            off = 0
            for k, (dt, shape, b) in items:
                e = {"name": k, "dtype": dt, "shape": shape}
                if shm is not None:  # 共有メモリに置く（64 バイトそろえ）
                    off = (off + 63) // 64 * 64
                    shm.buf[off : off + len(b)] = b
                    e["offset"] = off
                    off += len(b)
                else:
                    chunks.append(b)
                bins.append(e)
            head["bin"] = bins
        if self._shm is not None:
            head["shm"] = {"name": self._shm.name}
        data = json.dumps(head, separators=(",", ":")).encode() + b"\n"
        if sum(len(c) for c in chunks) < 1 << 16:
            self._send(data + b"".join(chunks))
        else:  # 大きい配列は写さずに送る
            self._send(data)
            for c in chunks:
                self._send(c)
        line = self._r.readline()
        if not line:
            raise ConnectionError("サーバーが接続を閉じた")
        h = json.loads(line)
        out = {}
        for b in h.get("bin", []):
            n = 1
            for s in b["shape"]:
                n *= s
            if "offset" in b:  # 共有メモリから写す（次の要求で上書きされるので）
                a = _np.frombuffer(self._shm.buf, dtype=b["dtype"], count=n, offset=b["offset"])
                out[b["name"]] = a.reshape(b["shape"]).copy()
            else:
                out[b["name"]] = self._read_array(b["dtype"], b["shape"], n)
        self.elapsed_ms = h.get("elapsed_ms")
        if "error" in h:
            e = h["error"]
            raise RpcError(e.get("code", -32000), e.get("message", ""), (e.get("data") or {}).get("kind"))
        return h.get("result"), out

    def _send(self, data):
        if self._sock is not None:
            self._sock.sendall(data)
            return
        m = memoryview(data)
        while len(m):
            k = self._f.write(m)
            m = m[k:]

    def _read_array(self, dtype: str, shape: list, n: int):
        nb = n * _ITEM[dtype][1]
        if _np is not None:
            a = _np.empty(shape, dtype=dtype)
            buf = memoryview(a.reshape(-1).view(_np.uint8))
        else:
            a = None
            buf = memoryview(bytearray(nb))
        got = 0
        while got < nb:  # 大きい配列は直接受け取る（写さない）
            k = self._r.readinto(buf[got:])
            if not k:
                raise ConnectionError("配列の途中で接続が切れた")
            got += k
        return a if a is not None else _from_bytes(dtype, shape, bytes(buf))

    # ---- 系 ----

    def ping(self) -> dict:
        return self.call("ping")[0]

    def load(
        self, file, *, cfg=None, state=None, beam_path=None, glass_dirs=None, lens_catalogs=None,
        glass_dir=None, lens_catalog=None,
    ) -> dict:
        """系を読み込む（前の系と set で変えた値は捨てる）。glass_dir・lens_catalog は旧名"""
        glass_dirs = _renamed(glass_dirs, glass_dir, "glass_dirs", "glass_dir")
        lens_catalogs = _renamed(lens_catalogs, lens_catalog, "lens_catalogs", "lens_catalog")
        p = {"file": os.fspath(file), "glass_dirs": _paths(glass_dirs), "lens_catalogs": _paths(lens_catalogs)}
        if cfg is not None:
            p["cfg"] = cfg
        if state is not None:
            p["state"] = state
        if beam_path is not None:
            p["beam_path"] = beam_path
        return self.call("load", p)[0]

    def reload(self) -> dict:
        """同じ読み方でファイルを読み直す（別アプリが系のファイルを書き換えた後に）"""
        return self.call("reload")[0]

    def system(self) -> dict:
        return self.call("system")[0]

    def get(self, *specs: str):
        """パラメータ（"3.t"・"STOP.r"・"G1.shift" など）の値。1 つなら数、複数なら並び"""
        v = self.call("get", {"params": list(specs)})[0]["values"]
        return v[0] if len(specs) == 1 else v

    def set(self, values: dict | None = None, **kw) -> list:
        """パラメータを変える: set({"3.t": 1.5, "5.dx": 0.01})。変えた名前の並びを返す"""
        v = dict(values or {})
        v.update(kw)
        return self.call("set", {"values": v})[0]["changed"]

    def write(self, path) -> str:
        """今の系を .zmx に書き出す（別アプリに渡す）"""
        return self.call("write", {"path": os.fspath(path)})[0]["written"]

    def save(self, path) -> dict:
        """今の系を系ファイル（.json）に書く。written・system_hash を返す"""
        return self.call("save", {"path": os.fspath(path)})[0]

    def edit(self, *ops: dict) -> dict:
        """系を編集する: edit({"op": "insert_surface", "at": 3, "surface": {"t": 0}}, ...)。
        どれかが失敗すれば系は変えない。notes・surfaces・system_hash を返す"""
        return self.call("edit", {"ops": list(ops)})[0]

    def surface(self, surface=None) -> dict:
        """面の値（surface は番号か名前、省くと全部の面）"""
        p = {} if surface is None else {"surface": surface}
        return self.call("surface", p)[0]

    # ---- 解析 ----

    def analyze(self, cmd: str, **kw) -> dict:
        """CLI の解析のサブコマンドを実行する。引数の名前は CLI の旗の - を _ にしたもの、値は JSON の型のまま
        （数の並びはリスト、旗は True）: analyze("mtf", field=2, freqs=[10, 20]) は
        lensbench mtf <系> --field 2 --freqs 10,20 と同じ結果。知らない名前・型違いは誤り"""
        p = {"cmd": cmd}
        p.update(kw)
        return self.call("analyze", p)[0]

    def info(self) -> dict:
        return self.analyze("info")

    def trace(
        self, px, py, *, field=None, hx=None, hy=None, wave=None, last=None, globals=False, global_=None
    ) -> dict:
        """正規化瞳座標 (px, py) の光線を追う。field（1 始まり）か (hx, hy)、wave は 1 始まり（省くと主波長）。
        返す dict: x y z l m n opl（最後の面の局所座標、globals なら大域）、err vig（0 なら通った）と
        rays・ok・hx・hy・wave・last。global_ は globals の旧名（lensbench.System.trace と同じ名前に）"""
        if global_ is not None:
            if globals:
                raise ValueError("globals と global_（旧名）の両方は渡せない")
            warnings.warn("global_ は globals の旧名（次の major 版で消す）", DeprecationWarning, stacklevel=2)
            globals = global_
        p = {"global": bool(globals)}
        for k, v in (("field", field), ("hx", hx), ("hy", hy), ("wave", wave), ("last", last)):
            if v is not None:
                p[k] = v
        res, arr = self.call("trace", p, {"px": px, "py": py})
        res.update(arr)
        return res

    def echo(self, **arrays):
        """配列をそのまま返してもらう（通信の時間を測る）"""
        return self.call("echo", {}, arrays)[1]

    def plugins(self) -> list:
        """登録された解析・検出器のプラグインの一覧（name・kind・doc・config・inputs・outputs）"""
        return self.call("plugins")[0]["plugins"]

    def plugin(self, name: str, config: dict | None = None, **inputs) -> dict:
        """解析・検出器のプラグインを読み込んだ系で 1 回呼ぶ。config は設定（数・文字列・数の並び）、
        inputs は入力の配列（numpy なら float64・complex128・bool のまま送る）。出力の配列の dict を返す"""
        res, arr = self.call("plugin", {"name": name, "config": config or {}}, inputs)
        return arr

    def fringe(self, **kw) -> dict:
        """単色の干渉縞（CLI の fringe と同じ引数・結果）。光束は beam1・beam2"""
        return self.analyze("fringe", **kw)

    # ---- 終わり ----

    # ---- 進み具合・中断（処理している要求を待たずに答える。回している接続とは別の接続から送る） ----

    def another(self) -> "Client":
        """同じサーバーへの別の接続（長い要求を回している間に progress・cancel・stop を送るため）。閉じてもサーバーは止めない"""
        a = self._addr
        return Client(a["host"], a["port"], a["timeout"], pipe=a["pipe"])

    def progress(self) -> dict:
        """running（要求を処理しているか）・optimize（反復ごとの評価関数の値）・tolerance・swap（(終えた数, 全体の数)、走っていなければ None）"""
        return self.call("progress")[0]

    def cancel(self) -> bool:
        """処理している解析に途中でやめるよう頼む（解析は「中断した」の誤りで返る）。最適化・公差・差し替え探索は stop で止める"""
        return self.call("cancel")[0]

    def stop(self, what: str) -> bool:
        """最適化（"optimize"）・公差（"tolerance"）・差し替え探索（"swap"）を止め、そこまでの結果を返させる"""
        return self.call("stop", {"what": what})[0]

    def shutdown(self):
        """サーバーを止める"""
        try:
            self.call("shutdown")
        except (ConnectionError, OSError):
            pass

    def close(self):
        if self._proc is not None:
            self.shutdown()
        try:
            self._r.close()
            if self._sock is not None:
                self._sock.close()
            if self._f is not None:
                self._f.close()
        except OSError:
            pass
        self._close_shm()
        if self._proc is not None:
            try:
                self._proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self._proc.kill()
                self._proc.wait()
            if self._proc.stdout:
                self._proc.stdout.close()
            self._proc = None
        if self._pipe_path:
            try:
                os.remove(self._pipe_path)
            except OSError:
                pass
            self._pipe_path = None

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


def _open_win_pipe(name: str, timeout: float):
    """Windows の名前付きパイプを開く（口が空くまで待つ）"""
    path = name if name.startswith("\\\\.\\pipe\\") else "\\\\.\\pipe\\" + name
    t0 = time.monotonic()
    while True:
        try:
            return open(path, "r+b", buffering=0)
        except OSError:
            # 口がふさがっている（ERROR_PIPE_BUSY、前の接続の直後）か、まだ作っていない。少し待って開き直す
            if time.monotonic() - t0 > timeout:
                raise
            time.sleep(0.01)


def _renamed(new, old, nn: str, on: str):
    """新しい名前か旧名の引数（両方は誤り）"""
    if new is not None and old is not None:
        raise ValueError(f"{nn} と {on}（旧名）の両方は渡せない")
    if old is not None:
        warnings.warn(f"{on} は {nn} の旧名（次の major 版で消す）", DeprecationWarning, stacklevel=3)
        return old
    return new


def _paths(v) -> list:
    if v is None:
        return []
    if isinstance(v, (str, os.PathLike)):
        return [os.fspath(v)]
    return [os.fspath(x) for x in v]


def _readline_timeout(p: subprocess.Popen, timeout: float) -> bytes:
    """起動した serve の最初の行（ポート）を待つ"""
    import threading

    box = []
    t = threading.Thread(target=lambda: box.append(p.stdout.readline()), daemon=True)
    t.start()
    t.join(timeout)
    if not box:
        p.kill()
        raise RuntimeError(f"lensbench serve が {timeout} 秒で起動しない")
    return box[0]


def timed(f, n: int = 100) -> float:
    """f() を n 回呼んだ 1 回あたりの時間 [ms]"""
    t0 = time.perf_counter()
    for _ in range(n):
        f()
    return (time.perf_counter() - t0) / n * 1e3

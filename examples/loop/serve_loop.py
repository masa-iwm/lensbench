"""解析ループの例: 別プロセスの lensbench に系を読み込ませたまま、間隔を変えて再解析を繰り返す。

    python examples/loop/serve_loop.py [lensbench の実行ファイル（省くと target/release か PATH）]

1. lensbench serve を起動して系を読み込む（1 回だけ）
2. 外で値を決める → set で系を変える → analyze・trace で再解析 → 結果を読む、を繰り返す
3. 1 回の往復の時間（通信だけ・小さい解析・数 MB の配列の追跡）を測って出す

別アプリが系のファイルを書き換える流れなら set の代わりに reload を呼ぶ（ファイルだけで回すなら lensbench watch）。
"""

import math
import os
import sys
import time

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
sys.path.insert(0, os.path.join(ROOT, "clients", "python"))

from lensbench_client import Client, timed  # noqa: E402


def main():
    try:  # Windows のコンソール（cp1252 など）でも日本語を出す
        sys.stdout.reconfigure(encoding="utf-8")
    except (AttributeError, ValueError):
        pass
    exe = sys.argv[1] if len(sys.argv) > 1 else None
    if exe is None and not os.environ.get("LENSBENCH_EXE"):
        for name in ("lensbench", "lensbench.exe"):  # ビルドしたもの（無ければ PATH の lensbench）
            p = os.path.join(ROOT, "target", "release", name)
            if os.path.exists(p):
                exe = p
    zmx = os.path.join(ROOT, "tests", "refs", "A0", "system.zmx")
    glass = os.path.join(ROOT, "tests", "glass")
    with Client.spawn(zmx, exe=exe, glass_dirs=[glass], threads=1) as lb:
        t0 = lb.get("3.t")
        print(f"系: {lb.system()['surfaces']} 面, 面 3 の厚さ {t0}")

        # 外で値を変える → 再解析 → 結果を読む
        print("   t3      RMS(視野1)   RMS(視野3)")
        for k in range(11):
            t = t0 + 0.2 * (k - 5)
            lb.set({"3.t": t})
            s1 = lb.analyze("spot", field=1)["spot"][0]["rms_centroid"]
            s3 = lb.analyze("spot", field=3)["spot"][0]["rms_centroid"]
            print(f"{t:7.3f}  {s1:.6e}  {s3:.6e}")
        lb.set({"3.t": t0})

        # 1 回の往復の時間
        n = 128
        g = [(-1 + 2 * (i + 0.5) / n, -1 + 2 * (j + 0.5) / n) for j in range(n) for i in range(n)]
        g = [(x, y) for x, y in g if x * x + y * y <= 1]
        px = [x for x, _ in g]
        py = [y for _, y in g]
        try:
            import numpy as np

            px, py = np.array(px), np.array(py)
            big = np.sin(np.arange(500_000.0))  # 4 MB
        except ImportError:
            big = [math.sin(i) for i in range(500_000)]
        ping = timed(lb.ping, 200)
        spot = timed(lambda: lb.analyze("spot", field=1), 50)
        tr = timed(lambda: lb.trace(px, py, field=2), 20)
        tr_server = lb.elapsed_ms
        echo = timed(lambda: lb.echo(a=big), 20)
        print(f"往復: ping {ping:.3f} ms, spot {spot:.3f} ms, "
              f"trace {len(px)} 本 {tr:.3f} ms（サーバー内 {tr_server:.3f} ms）, echo 4 MB {echo:.3f} ms")

        # 変えてから戻すと最初と同じ結果（像面距離の解きは解き直すので 1e-14 程度の差はありうる）
        a = lb.analyze("spot", field=2)["spot"][0]["rms_centroid"]
        lb.set({"3.t": t0 + 1})
        lb.set({"3.t": t0})
        b = lb.analyze("spot", field=2)["spot"][0]["rms_centroid"]
        assert abs(a - b) <= 1e-12 * abs(a), (a, b)
    compare_transports(exe, zmx, glass)
    print("ok", time.strftime("%H:%M:%S"))


def compare_transports(exe, zmx, glass):
    """TCP・パイプ・共有メモリで 100 万本の trace（入 16 MB・出 72 MB）の往復の時間を比べる"""
    try:
        import numpy as np
    except ImportError:
        return
    px = np.linspace(-1, 1, 1_000_000)
    py = 0.3 * px
    shm_ok = sys.platform in ("win32", "linux")
    for pipe in (False, True):
        for shm in (False, True) if shm_ok else (False,):
            with Client.spawn(zmx, exe=exe, glass_dirs=[glass], threads=1, pipe=pipe) as lb:
                if shm:
                    lb.use_shm(128 << 20)
                lb.trace(px, py, field=2)
                best, server = math.inf, math.inf
                for _ in range(5):
                    t = time.perf_counter()
                    lb.trace(px, py, field=2)
                    best = min(best, (time.perf_counter() - t) * 1e3)
                    server = min(server, lb.elapsed_ms)
                name = ("パイプ" if pipe else "TCP") + (" + 共有メモリ" if shm else "")
                print(f"trace 100 万本 {name}: {best:.0f} ms（サーバー内 {server:.0f} ms、通信 {best - server:.0f} ms）")


if __name__ == "__main__":
    main()

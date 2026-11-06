"""BS の経路・状態ごとの多重構成の .zmx（lensbench）の基準を OpticStudio で作る（Zemax PC で実行。ZOS-API は zosconn.py、pythonnet）。

このフォルダの *.zmx（lensbench の `zmx_write::save_configs` で書いたもの。examples/bs_configs.rs）を開き、
構成ごとに像面までの実光線（NormUnpol）: 全波長・全視野・下の瞳の点で、像面の局所の X Y Z L M N と光路長
（OPDMode None の opd）、err、vig を cfg_refs.json に書く。

  set LB_ZOSCONN_DIR=（zosconn.py のあるフォルダ）
  python make_cfg_refs.py

ガラスは LBSCH（試験の tests/glass/SCHOTT.AGF を OpticStudio の Glasscat に LBSCH.AGF として入れたもの）。
"""
import glob
import json
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.environ.get('LB_ZOSCONN_DIR', HERE))

PUPIL = [(0.0, 0.0), (0.7, 0.2), (-0.5, 0.6), (0.3, -0.9), (0.0, -1.0), (1.0, 0.0)]


def norm_fields(S):
    fs = S.SystemData.Fields
    pts = [(fs.GetField(i).X, fs.GetField(i).Y) for i in range(1, fs.NumberOfFields + 1)]
    m = max(math.hypot(x, y) for x, y in pts) or 1.0
    return [(x / m, y / m) for x, y in pts]


def unpol(S, Z, ima):
    out = []
    nw = S.SystemData.Wavelengths.NumberOfWavelengths
    for w in range(nw):
        for fi, (hx, hy) in enumerate(norm_fields(S)):
            rt = S.Tools.OpenBatchRayTrace()
            nu = rt.CreateNormUnpol(len(PUPIL), Z.Tools.RayTrace.RaysType.Real, int(ima))
            for (px, py) in PUPIL:
                nu.AddRay(w + 1, hx, hy, px, py, getattr(Z.Tools.RayTrace.OPDMode, 'None'))
            rt.RunAndWaitForCompletion()
            nu.StartReadingResults()
            for (px, py) in PUPIL:
                r = nu.ReadNextResult()
                out.append({'wave': w + 1, 'field': fi + 1, 'hx': hx, 'hy': hy, 'px': px, 'py': py,
                            'ok': bool(r[0]), 'err': int(r[2]), 'vig': int(r[3]),
                            'xyz': [r[4], r[5], r[6]], 'lmn': [r[7], r[8], r[9]], 'opl': r[13]})
            rt.Close()
    return out


def main():
    import zosconn
    S, Z = zosconn.sys_, zosconn.ZOSAPI
    res = {'zos_version': '%s.%s' % (zosconn.app.ZOSMajorVersion, zosconn.app.ZOSMinorVersion),
           'pupil': PUPIL, 'cases': {}}
    try:
        for zmx in sorted(glob.glob(os.path.join(HERE, '*.zmx'))):
            name = os.path.splitext(os.path.basename(zmx))[0]
            if not S.LoadFile(zmx, False):
                raise SystemExit('開けない: %s' % zmx)
            mce = S.MCE
            ima = S.LDE.NumberOfSurfaces - 1
            cfgs = []
            for k in range(1, mce.NumberOfConfigurations + 1):
                mce.SetCurrentConfiguration(k)
                stop = S.LDE.StopSurface
                rays = unpol(S, Z, ima)
                bad = sum(1 for r in rays if not (r['ok'] and r['err'] == 0 and r['vig'] == 0))
                cfgs.append({'config': k, 'stop': stop, 'unpol': rays})
                print(name, 'cfg', k, 'stop', stop, 'rays', len(rays), 'bad', bad, flush=True)
            res['cases'][name] = {'surfaces': ima + 1, 'configs': cfgs}
        json.dump(res, open(os.path.join(HERE, 'cfg_refs.json'), 'w'), indent=1)
        print('-> cfg_refs.json')
    finally:
        zosconn.close()


if __name__ == '__main__':
    main()

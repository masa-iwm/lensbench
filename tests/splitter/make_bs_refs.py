"""ビームスプリッター（lensbench）の基準を OpticStudio で作る（Zemax PC で実行。ZOS-API は zosconn.py、pythonnet）。

このフォルダの *.zmx（lensbench が書いた各経路の系。反射の面は膜なしの MIRROR）を開き、
1) 像面までの実光線（NormUnpol）: 全波長・全視野・下の瞳の点で、像面の局所の X Y Z L M N と光路長（OPDMode None の opd）、err、vig
2) cube_T・plate_T だけ: 偏光（NormPol、入力 x・y）の像面の電場 Ex Ey Ez（複素）
3) cube_T の分ける面（N-BK7|N-BK7、膜 BS3）と plate_T の表（AIR|N-BK7、膜 BS3）の Reflection vs Angle（主波長、0〜80°、1° 刻み）
を bs_refs.json と reflect_*.txt に書く。

  set LB_ZOSCONN_DIR=（zosconn.py のあるフォルダ）
  python make_bs_refs.py

膜ファイル LBBS.DAT を OpticStudio の Coatings フォルダにコピーしておく（.zmx の COFN が LBBS.DAT）。
ガラスは OpticStudio の SCHOTT カタログ（N-BK7）。
"""
import glob
import json
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.environ.get('LB_ZOSCONN_DIR', HERE))

PUPIL = [(0.0, 0.0), (0.7, 0.2), (-0.5, 0.6), (0.3, -0.9), (0.0, -1.0), (1.0, 0.0)]
FIELDS = [(0.0, 0.0), (0.0, 1.5), (1.0, -1.0)]   # 物体の高さ [mm]。正規化は最大の半径 1.5
POL = ['cube_T', 'plate_T']


def norm_fields():
    m = max((x * x + y * y) ** 0.5 for x, y in FIELDS)
    return [(x / m, y / m) for x, y in FIELDS]


def unpol(S, Z, nw, ima):
    out = []
    for w in range(nw):
        for (hx, hy) in norm_fields():
            rt = S.Tools.OpenBatchRayTrace()
            nu = rt.CreateNormUnpol(len(PUPIL), Z.Tools.RayTrace.RaysType.Real, int(ima))
            for (px, py) in PUPIL:
                nu.AddRay(w + 1, hx, hy, px, py, getattr(Z.Tools.RayTrace.OPDMode, 'None'))
            rt.RunAndWaitForCompletion()
            nu.StartReadingResults()
            for (px, py) in PUPIL:
                r = nu.ReadNextResult()
                out.append({'wave': w + 1, 'hx': hx, 'hy': hy, 'px': px, 'py': py,
                            'ok': bool(r[0]), 'err': int(r[2]), 'vig': int(r[3]),
                            'xyz': [r[4], r[5], r[6]], 'lmn': [r[7], r[8], r[9]], 'opl': r[13]})
            rt.Close()
    return out


def pol(S, Z, nw, ima):
    out = []
    for w in range(nw):
        for (hx, hy) in norm_fields():
            for m, (ex, ey) in enumerate([(1.0, 0.0), (0.0, 1.0)]):
                rt = S.Tools.OpenBatchRayTrace()
                nu = rt.CreateNormPol(len(PUPIL), Z.Tools.RayTrace.RaysType.Real, 1.0, 0.0, 0.0, 0.0, int(ima))
                for (px, py) in PUPIL:
                    nu.AddRay(w + 1, hx, hy, px, py, ex, 0.0, ey, 0.0, 0.0, 0.0)
                rt.RunAndWaitForCompletion()
                nu.StartReadingResults()
                for (px, py) in PUPIL:
                    r = nu.ReadNextResult()
                    out.append({'wave': w + 1, 'hx': hx, 'hy': hy, 'px': px, 'py': py, 'in': m,
                                'ok': bool(r[0]), 'err': int(r[2]),
                                'E': [[r[3], r[4]], [r[5], r[6]], [r[7], r[8]]], 'intensity': r[9]})
                rt.Close()
    return out


def reflect_vs_angle(S, Z, surf, path):
    """Analyze > Coatings > Reflection vs Angle（面 surf、主波長）のテキスト"""
    a = S.Analyses.New_Analysis(Z.Analysis.AnalysisIDM.ReflectionvsAngle)
    st = Z.Analysis.Settings.Coatings.IAS_RTA_Angle(a.GetSettings())
    st.Surface.SetSurfaceNumber(int(surf))
    st.Wavelength.UseAllWavelengths = False
    # OS 26.1: この解析は SetWavelengthNumber(n) で n+1 番目の波長になる（1→0.5876, 2/3→0.6563）。主波長 2 番を得るため 1 を渡し、下でテキストの見出しを確かめる
    st.Wavelength.SetWavelengthNumber(1)
    st.MinAngle = 0.0
    st.MaxAngle = 80.0
    a.ApplyAndWaitForCompletion()
    f = os.path.join(tempfile.gettempdir(), 'lb_bs_reflect.txt')
    if os.path.exists(f):
        os.remove(f)
    a.GetResults().GetTextFile(f)
    for enc in ('utf-16', 'utf-8', 'cp1252'):
        try:
            txt = open(f, encoding=enc).read()
            break
        except UnicodeError:
            continue
    wl = S.SystemData.Wavelengths.GetWavelength(2).Wavelength
    if ('Wavelength: %.4f' % wl) not in txt:
        raise RuntimeError('Reflection vs Angle の波長が主波長 %.4f でない' % wl)
    open(path, 'w', encoding='utf-8').write(txt)
    a.Close()


def find_split(S):
    """膜 BS3 の付いた面（最初のもの）"""
    lde = S.LDE
    for k in range(1, lde.NumberOfSurfaces):
        if lde.GetSurfaceAt(k).Coating == 'BS3':
            return k
    raise RuntimeError('BS3 の面がない')


def main():
    import zosconn
    S, Z = zosconn.sys_, zosconn.ZOSAPI
    res = {'zos_version': '%s.%s' % (zosconn.app.ZOSMajorVersion, zosconn.app.ZOSMinorVersion),
           'pupil': PUPIL, 'fields': norm_fields(), 'cases': {}}
    try:
        for zmx in sorted(glob.glob(os.path.join(HERE, '*.zmx'))):
            name = os.path.splitext(os.path.basename(zmx))[0]
            if not S.LoadFile(zmx, False):
                raise SystemExit('開けない: %s' % zmx)
            nw = S.SystemData.Wavelengths.NumberOfWavelengths
            ima = S.LDE.NumberOfSurfaces - 1
            c = {'unpol': unpol(S, Z, nw, ima)}
            if name in POL:
                c['pol'] = pol(S, Z, nw, ima)
                k = find_split(S)
                reflect_vs_angle(S, Z, k, os.path.join(HERE, 'reflect_%s.txt' % name))
                c['split_surface'] = k
            res['cases'][name] = c
            bad = sum(1 for r in c['unpol'] if not (r['ok'] and r['err'] == 0 and r['vig'] == 0))
            print(name, 'rays', len(c['unpol']), 'bad', bad, flush=True)
        json.dump(res, open(os.path.join(HERE, 'bs_refs.json'), 'w'), indent=1)
        print('-> bs_refs.json')
    finally:
        zosconn.close()


if __name__ == '__main__':
    main()

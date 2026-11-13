#!/usr/bin/env python3
"""テスト用ガラスカタログ tests/glass/*.AGF を refractiveindex.info のデータベース（CC0）から作る。

    git clone --depth 1 https://github.com/polyanskiy/refractiveindex.info-database ri
    python3 scripts/make_test_glass.py ri

基準の光学系（tests/refs）が使うガラスだけを、分散式（Sellmeier、AGF の式 2）・温度係数・波長範囲で書く。
"""
import os, sys, yaml

GLASSES = {'SCHOTT': ['N-SK16', 'F2', 'N-BK7', 'N-LAK9'], 'OHARA': ['S-FSL5']}
OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), 'tests', 'glass')


def entry(path, name):
    y = yaml.safe_load(open(path, encoding='utf-8'))
    f = [d for d in y['DATA'] if d['type'] == 'formula 2']
    if not f:
        raise SystemExit('%s: Sellmeier (formula 2) がない' % path)
    c = [float(v) for v in f[0]['coefficients'].split()]
    if c[0] != 0:
        raise SystemExit('%s: 定数項がある' % path)
    lo, hi = (float(v) for v in f[0]['wavelength_range'].split())
    pr = y['PROPERTIES']
    td = ([float(v) for v in pr['thermal_dispersion'][0]['coefficients'].split()]
          if 'thermal_dispersion' in pr else [0.0] * 6)
    tref = y.get('CONDITIONS', {}).get('temperature', 293) - 273.0
    nd, vd = pr['nd'], pr['Vd']
    code = pr.get('glass_code', 0)
    return ['NM %s 2 %s %.5f %.2f 0 1' % (name, code, nd, vd),
            'CD ' + ' '.join('%.9E' % v for v in c[1:7]) + ' 0.0 0.0',
            'TD ' + ' '.join('%.6E' % v for v in td) + ' %.1f' % tref,
            'LD %.5E %.5E' % (lo, hi)]


def main(ri):
    os.makedirs(OUT, exist_ok=True)
    for cat, names in GLASSES.items():
        lines = ['CC lensbench test catalog: %s glasses from the refractiveindex.info database (CC0 1.0),' % cat,
                 'CC https://github.com/polyanskiy/refractiveindex.info-database (database/data/specs/%s/optical)' % cat.lower()]
        for n in names:
            lines += entry(os.path.join(ri, 'database', 'data', 'specs', cat.lower(), 'optical', n + '.yml'), n)
        with open(os.path.join(OUT, cat + '.AGF'), 'w', encoding='ascii', newline='\r\n') as f:
            f.write('\n'.join(lines) + '\n')
        print('wrote', os.path.join(OUT, cat + '.AGF'))


if __name__ == '__main__':
    main(sys.argv[1])

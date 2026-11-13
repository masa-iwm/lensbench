#!/usr/bin/env python3
"""GUI に同梱するガラスカタログ crates/lensbench-gui/assets/glass/*.AGF を refractiveindex.info のデータベース（CC0）から作る
（GUX-01: 何も設定していなくても普通の .zmx が開けるように）。

    git clone --depth 1 https://github.com/polyanskiy/refractiveindex.info-database ri
    python3 scripts/make_builtin_glass.py ri

SCHOTT・OHARA の optical のうち、分散式が Sellmeier（AGF の式 2）のガラスを全部書く。温度係数の無いガラスは 0。
"""
import glob, os, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import make_test_glass as mt  # noqa: E402

OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                   'crates', 'lensbench-gui', 'assets', 'glass')


def main(ri):
    os.makedirs(OUT, exist_ok=True)
    for cat in ['SCHOTT', 'OHARA']:
        lines = ['CC lensbench built-in catalog: %s glasses from the refractiveindex.info database (CC0 1.0),' % cat,
                 'CC https://github.com/polyanskiy/refractiveindex.info-database (database/data/specs/%s/optical).' % cat.lower(),
                 'CC Sellmeier (formula 2) glasses only. Missing thermal coefficients are written as 0.']
        n = 0
        for path in sorted(glob.glob(os.path.join(ri, 'database', 'data', 'specs', cat.lower(), 'optical', '*.yml'))):
            name = os.path.splitext(os.path.basename(path))[0]
            try:
                lines += mt.entry(path, name)
                n += 1
            except SystemExit:
                continue
        with open(os.path.join(OUT, cat + '.AGF'), 'w', encoding='ascii', newline='\r\n') as f:
            f.write('\n'.join(lines) + '\n')
        print('wrote', os.path.join(OUT, cat + '.AGF'), n, 'glasses')


if __name__ == '__main__':
    main(sys.argv[1])

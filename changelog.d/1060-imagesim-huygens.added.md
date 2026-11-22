- **像の模擬の Huygens PSF**: `image-sim` の `--aberrations huygens`（Python `aberrations="huygens"`）。PSF の格子の各点で Huygens PSF
  （像面の上、GPU があれば GPU で足す）を畳み込む。像面が傾いた系や斜めの視野で、FFT PSF の一次の写像の近似を使わない

- **全部けられる視野**: 視野の光線が全部けられる（機構部品の既定のけられ・開口）と、`mtf`・`psf`・`huygens-mtf`・`geo-mtf`・
  `wavefront`・`zernike` が解析全体の誤りで終わっていたのを、その視野だけを注意（`field N: …`）にしてほかの視野の結果を出すよう
  直した（`spot` と同じ扱い。CLI・serve・GUI・Python の `analyze`）。誤りの符号 `system.all_vignetted` を足した

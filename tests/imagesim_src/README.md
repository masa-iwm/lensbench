# Image Simulation references

Input test patterns for the OpticStudio Image Simulation references in
`../refs/A0/imagesim_os/` and `../refs/D0/imagesim_os/`.
They live outside `tests/refs/` because every folder there is a system (tests walk them and open `system.zmx`).
The references come from OpticStudio 2026 R1.01 (26.1.1), run through the standalone ZOS-API.

## Inputs (512 x 512, `[row, col]`, row 0 = top)
- `lb937_gray.png`: white on black.
  - 1 px grid lines at rows and columns 32 + 64k.
  - 1 px dots at (64j, 64k), j, k = 1..7.
  - An orientation marker: an 8 x 8 square at rows/cols 10..17, plus a bar at rows 10..12, cols 100..123.
- `lb937_rgb.png`: the same dot grid, with dot (64j, 64k) in channel R/G/B = (j + k) % 3. Same white marker.
- `lb937_gray127.png`: `lb937_gray` halved (max 127).

OpticStudio was given 24-bit BMPs with identical pixels.

## Per-run files
Each `imagesim_os/<run>.json` records:
- the settings read back after they were set: FieldHeight, centre field, Aberrations, PSF grid, Oversampling, PixelSize/XPixels/YPixels, and the wavelength mode;
- the EFL, the effective pixel size, and the time spent in ApplyAndWaitForCompletion;
- the real chief-ray image heights (REAX/REAY) along x and y.

`<run>.png` is the OpticStudio output file, unchanged.

## Facts measured with these runs
- **No numeric arrays from ZOS-API.** The results object has no data grids and no series, and `GetTextFile` returns False. The only output is the `OutputFile` bitmap. PNG, BMP and JPG are written; IMA and TXT come out empty. The bitmap is 8 bit.
- **Brightness is not normalised to the max.** Output is about input value x relative illumination, clipped at 255. A max-127 input gives a max-127 output (126 on D0).
- **Single wavelength (`SetWavelengthNumber`):** only one channel is filled. Wavelength 1 goes to B, 2 to G, 3 to R. Not checked: whether this follows the wavelength number or its colour. RGB mode fills all three channels.
- **Geometry**
  - FieldHeight is the full image height. The input pixels are linear in field angle, and the image centre is pixel 256.
  - The output has the same pixel count as the input when XPixels/YPixels = 0.
  - The default pixel size is EFL x FieldHeight[rad] / 512 (input rows). Setting that value explicitly gives a bit-identical image.
- **Orientation:** the output keeps the input orientation, with no flip or transpose. Top = +Y field, right = +X field. This was checked with off-axis centre fields 2 and 4.
- **Blur:** with Oversampling None, the PSF is applied at input-pixel resolution. On these systems Diffraction, Geometric and None are identical when the PSF is smaller than one input pixel (D0 everywhere; A0 Geometric = Diffraction). Oversampling X8+ or a small FieldHeight brings out the Airy blur, e.g. D0 `gray_diff_ov8`.
- **FieldHeight used:** the max field value (A0 20, D0 10), so the image covers only ±10° / ±5°.
- **Timing:** without oversampling, each run takes 0.1–5 s. D0 Diffraction takes 12.8 s at X8 and 151 s at X16.
- **Open:** in D0 `gray_none_f2` the right-hand dots sit 1 px inside their symmetric positions.

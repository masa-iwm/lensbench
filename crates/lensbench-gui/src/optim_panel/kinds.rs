//! 項の種類の説明（ツールチップ）と、足したときの値

use crate::i18n::tr;

/// 種類の説明（ツールチップ）
pub(super) fn type_hint(ty: &str) -> &'static str {
    match ty {
        "spot" => tr("RMS spot radius (field, wavelength; empty wavelength = polychromatic)"),
        "wavefront" => tr("RMS wavefront error [waves]"),
        "mtf" => tr("MTF − target (default target 1). kind: diffraction, square (square-wave response, like MSW*), geometric (like GMT*; diff_limit scales by the diffraction limit) or huygens (like MTH*). dir avg: mean of tan and sag (like MTFA)"),
        "energy" => tr("Enclosed energy − target: with size_at the size [lens units] that holds that fraction of the light (like GENC and DENC), with fraction_at the fraction inside that size (like GENF and DENF). method: geometric rays, fft or huygens PSF; airy multiplies the geometric circle by the Airy disk"),
        "param" => tr("Value of a system parameter (written like a variable: 6.t, 3.c, wave1, aper) in the term's configuration − target (like MCOV, MCOG and MCOL)"),
        "strehl" => tr("Strehl ratio from the FFT PSF − target (like STRH)"),
        "illum" => tr("Relative illumination to the axis (relative, like RELI) or effective F-number (fno, like EFNO) from the solid angle of the light reaching the image point − target"),
        "paraxial" => tr("System quantity such as a paraxial value − target (same values as System Data)"),
        "ray" => tr("Position / direction cosine of one real ray in the surface's local coordinates − target (like REAX–REAC; TRAX, OPTH, RAGX, RAID etc. by q)"),
        "centroid" => tr("x or y of the beam centroid on the image − target (same rays as the spot term; e.g. a compensator that centres the returned spot with a mirror tilt)"),
        "opd" => tr("OPD of one ray [waves] − target (like OPDC)"),
        "zernike" => tr("Zernike coefficient [waves] − target"),
        "coupling" => tr("Fiber coupling efficiency − target (default target 1, finite object)"),
        "thickness" => tr("Center thickness of surface surf (the sum up to surface to) − target (like CTVA and TTHI; min or max for CTGT and CTLT). edge: edge thickness (ETVA). each: compare each surface with min or max (MNCT, MNCA, MNEG etc.)"),
        "calc" => tr("op of args (numbers or terms with a single value, e.g. {\"type\": \"paraxial\", \"q\": \"efl\", \"target\": 0}, 5) − target (like SUMM, DIFF, PROD, DIVI, ABSO, SQRT and RECI)"),
        "tsag" => tr("Distance from point (x, y, z in the surface's local coordinates) to surface surf along the z axis turned by tilt (about x, y and z [deg]) − target (like TSAG). flat: the surface is flat outside the clear semi-diameter when the lens edge is larger (Mode 0)"),
        "bfs" => tr("Best-fit sphere of surface surf with the least material to remove, fitted from radius rmin to rmax (0 = clear semi-diameter) − target: q curv, radius, vertex, depth (largest), volume, slope (largest slope difference), depth_rms, slope_rms (like BFSD)"),
        "spread" => tr("Largest − smallest value of term (a term with a single value, e.g. {\"type\": \"thickness\", \"surf\": 1, \"to\": 5}) over all the configurations − target (with max like ZTHI)"),
        "surface" => tr("Curvature, conic, sag, thickness at a point, total thickness, diameter, global vertex position or direction, glass data or parameter of surface surf − target (like CVVA, COVA, SSAG, SAGX, STHI, TTVA, DMVA, GLCX, GLCA, GCOS and PMVA; min or max for CVGT, DMLT etc.). x and y are the point for sag and thick and the direction for total. With to: compare each surface with min or max and sum the excess (MNSD, MNIN etc.)"),
        "volume" => tr("Volume [cm³] of the elements from surface surf to to, air spaces included, or with mass the mass [g] of the glass elements − target (like VOLU and TMAS). clear: outer radius from the clear semi-diameters instead of the lens edges. cylinder: the smallest cylinder around the surfaces surf to to + 1 instead (like CVOL)"),
        "seidel" => tr("Seidel aberration coefficient [waves] of surface surf (0 = sum) − target: spha W040, coma W131, asti W222, fcur W220, dist W311 (like SPHA, COMA, ASTI, FCUR and DIST)"),
        "distortion" => tr("Real-ray distortion of the chief ray − target: percent of the predicted image height, or real − predicted with absolute. The prediction is linear from the reference field (the axis by default, like DIST of surface 0 and DISG); peak is the largest absolute value from the axis to the field (DIMX), ftheta the calibrated f-theta distortion (DISC)"),
        "longitudinal" => tr("Distance from the image surface to where an axial ray crosses the axis − target (zone 0 = paraxial marginal ray; like LONA). With wave2: the distance at wave2 − at wave (axial color, AXCL)"),
        "lateral_color" => tr("Height on the image of the paraxial chief ray of the largest field at wave2 − at wave − target (like LACL)"),
        "semi" => tr("Compare the semi-diameter of each surface from surf to to with min or max and sum the excess (like MNSD and MXSD)"),
        "fnum" => tr("Real-ray F-number 1/(n′(sin θ₊ + sin θ₋)) from the chief ray and the two rays at the pupil edge (y for tan, x for sag) − target (like TFNO and SFNO)"),
        "cardinal" => tr("Focal length or cardinal point of surfaces from to to − target (like CARD, EFLX and EFLY; empty from and to = surface 1 to the last surface before the image). Object-side points are measured from surface from, image-side points from surface to"),
        "plugin" => tr("Output output of a registered plugin − target (name is rms, zernike, …; args is the settings as JSON, e.g. {\"rings\": 4}; index picks one element)"),
        "comment" => tr("Comment row (like BLNK in OpticStudio); not part of the merit function"),
        "zplm" => tr("Value of OPTRETURN data from a ZPL macro − target (like ZPLM; macro is a number nn = ZPLnn.ZPL or a name, file is a .ZPL path, macro_dirs are folders to search separated by ;)"),
        _ => "",
    }
}

/// 種類ごとの、足したときの値
pub(super) fn defaults(ty: &str) -> Vec<(&'static str, &'static str)> {
    match ty {
        "spot" | "wavefront" | "opd" | "zernike" => vec![("field", "1")],
        "mtf" => vec![("field", "1"), ("freq", "30")],
        "energy" => vec![("field", "1"), ("size_at", "0.8"), ("target", "")],
        "strehl" => vec![("field", "1"), ("target", "1")],
        "param" => vec![("name", "1.t"), ("target", "")],
        "illum" => vec![("field", "1"), ("q", "relative"), ("target", "1")],
        "paraxial" => vec![("q", "efl"), ("target", "")],
        "ray" => vec![("field", "1"), ("py", "1"), ("q", "y"), ("target", "0")],
        "centroid" => vec![("field", "1"), ("q", "x"), ("target", "0")],
        "thickness" => vec![("surf", "1"), ("target", "")],
        "surface" => vec![("surf", "1"), ("q", "curv"), ("target", "")],
        "cardinal" => vec![("q", "efl"), ("target", "")],
        "fnum" => vec![("field", "1"), ("dir", "tan"), ("target", "")],
        "seidel" => vec![("q", "spha"), ("surf", "0"), ("target", "")],
        "distortion" => vec![("target", "")],
        "longitudinal" => vec![("zone", "0"), ("target", "")],
        "lateral_color" => vec![("wave", "1"), ("wave2", "3"), ("target", "")],
        "semi" => vec![("surf", "1"), ("max", "")],
        "volume" => vec![("surf", "1"), ("target", "")],
        "zplm" => vec![("macro", "0"), ("data", "0")],
        "plugin" => vec![("name", "rms"), ("field", "1"), ("output", "wfe")],
        "tsag" => vec![("surf", "1"), ("point", "0, 10, 0"), ("target", "")],
        "bfs" => vec![("surf", "1"), ("q", "curv"), ("target", "")],
        "spread" => vec![
            ("term", "{\"type\": \"thickness\", \"surf\": 1}"),
            ("max", "0"),
        ],
        _ => vec![],
    }
}

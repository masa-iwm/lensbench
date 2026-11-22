//! 熱の構成を作るコマンド（make-thermal、[`lensbench_core::thermal`]）

use super::*;

// `make-thermal` の引数と説明（main.rs を大きくしないようここに置く）
/// Make thermal configurations (OpticStudio's Make Thermal)
///
/// Configuration 1 is the system as it is, and one configuration is added for each temperature of --temps with the temperature, pressure, curvatures, thicknesses, aspheric coefficients and sizes expanded by the TCE of the glasses and of the air spaces (the TCE column of the surface). Writes the system with the multi-configuration table to --out (.json or .zmx)
#[derive(clap::Args)]
pub(crate) struct MakeThermalArgs {
    #[command(flatten)]
    pub(crate) c: Common,
    /// Temperatures of the added configurations [°C] (comma-separated)
    #[arg(
        long,
        value_delimiter = ',',
        required = true,
        allow_hyphen_values = true
    )]
    temps: Vec<f64>,
    /// Pressures of the added configurations [atm] (comma-separated, as many as the temperatures; the system pressure if omitted)
    #[arg(long, value_delimiter = ',')]
    pressures: Vec<f64>,
    /// File to write (.json or .zmx)
    #[arg(long, short)]
    out: PathBuf,
}

/// 今の構成を構成 1 にして熱の構成を作り、--out に書く
pub(crate) fn make_thermal(a: &MakeThermalArgs) -> Result<Value, AppError> {
    let s = load(&a.c)?;
    let spec = lensbench_core::thermal::ThermalSpec {
        temps: a.temps.clone(),
        pressures: a.pressures.clone(),
    };
    let th = lensbench_core::thermal::make_thermal(&s, s.configs().map_or(1, |x| x.active), &spec)?;
    for w in &th.warnings {
        eprintln!("{}", lensbench_core::i18n::warning(w));
    }
    let out = &a.out;
    if out
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("zmx"))
    {
        warn_notes(zmx_write::save(out, &th.system)?);
    } else {
        lensbench_project::sysfile::save(out, &th.system, &[])?;
        warn_notes(lensbench_project::sysfile::save_notes(&th.system));
    }
    Ok(json!({
        "written": out.display().to_string(),
        "configs": th.system.configs().map_or(1, |x| x.n),
        "warnings": th.warnings,
    }))
}

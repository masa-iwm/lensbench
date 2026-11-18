//! 型の要求（[`Request`]）を処理する。型を付けていない method は JSON の口（[`super::answer`]）へ

use super::*;
use lensbench_project::edit_spec::Ref;

/// 要求を 1 つ処理する（鍵・中断・panic は呼び手の [`super::dispatch_req`]）
pub(super) fn run(
    sess: &Session,
    req: Request,
    bins: Vec<Bin>,
) -> Result<(Answer, Vec<Bin>), RpcErr> {
    let v = match req {
        Request::Json { method, params } => return answer(&method, &params, bins),
        // 減衰最小二乗の最適化（GUI の最適化パネル）
        Request::Optimize(op) => {
            return Ok((Answer::Optimize(Box::new(optimize(sess, *op)?)), Vec::new()))
        }
        // カタログレンズの差し替え探索（GUI）
        Request::Swap(r) => return Ok((Answer::Swap(Box::new(swap(sess, &r)?)), Vec::new())),
        // モンテカルロ公差
        Request::Tolerance(r) => {
            return Ok((
                Answer::Tolerance(Box::new(tolerance(sess, &r)?)),
                Vec::new(),
            ))
        }
        // ズーム軌跡（GUI のズームのパネル）
        Request::Zoom(z) => return Ok((Answer::Zoom(Box::new(zoom(sess, &z)?)), Vec::new())),
        Request::Load(r) => {
            let spec = r.spec(sess)?;
            let l = read_file_notes(&spec)?;
            let mut v = summary(&l.sys, &spec, &l.notes, &l.blocks);
            store_loaded(sess, l, spec, r.history, false);
            add_history(sess, &mut v);
            v
        }
        // 同じ読み方でファイルを読み直す（set で変えた値は捨てる）。別アプリが系のファイルを書き換えた後に使う
        Request::Reload { history: h } => {
            let (_, spec) = sess.current()?;
            let l = read_file_notes(&spec)?;
            let mut v = summary(&l.sys, &spec, &l.notes, &l.blocks);
            store_loaded(sess, l, spec, h, true);
            add_history(sess, &mut v);
            v
        }
        Request::System => {
            let g = sess.served();
            let x = g
                .as_ref()
                .ok_or_else(|| RpcErr::app("no system (load first)"))?;
            summary(&x.sys, &x.spec, &x.notes, &x.blocks)
        }
        // 読み込んだ後の set・edit（最適化の結果を当てたものも）を 1 段ずつ元に戻す・やり直す（100 段まで。
        // 同じパラメータの set が 1 秒以内に続けば 1 段。load・reload で空になる。history: "push" で読めば読む前の系も 1 段）
        Request::Undo { steps } => step_history_n(sess, true, steps.max(1))?,
        Request::Redo { steps } => step_history_n(sess, false, steps.max(1))?,
        Request::History => history(sess),
        Request::Get(g) => {
            let (s, _) = sess.current()?;
            match g {
                GetReq::One(one) => json!({ "value": s.get_param(s.param(&one)?)? }),
                GetReq::Many(list) => {
                    let out = list
                        .iter()
                        .map(|spec| Ok(s.get_param(s.param(spec)?)?))
                        .collect::<Result<Vec<_>, RpcErr>>()?;
                    json!({ "values": out })
                }
            }
        }
        Request::Set { values } => {
            let (s, spec) = sess.current()?;
            let changes = parse_changes(&s, &values)?;
            let mut n = s.with_params(&changes)?;
            // 多重構成の表の行の値なら、表の今の構成の列も
            n.sync_active_config();
            let names: Vec<String> = changes.iter().map(|(q, _)| n.param_name(*q)).collect();
            let rec = changes
                .iter()
                .zip(&names)
                .map(|((q, v), name)| Ok((name.clone(), s.get_param(*q)?, *v)))
                .collect::<Result<Vec<_>, lensbench_core::Error>>()?;
            store_changed(sess, n, spec, Change::Set(rec));
            let mut v = history(sess);
            v["changed"] = json!(names);
            v
        }
        // 系を .zmx に書き出す（別アプリに渡す）
        Request::Write { path } => {
            let (s, _) = sess.current()?;
            let notes = zmx_write::save(&sess.files.check_path(&path, true)?, &s)?;
            json!({ "written": path, "notes": notes })
        }
        // 系の編集: ops の操作の列を順に当てる（どれかが失敗すれば系は変えない）
        Request::Edit { ops } => {
            let (s, spec) = sess.current()?;
            let mut n = (*s).clone();
            let notes = lensbench_project::edit_ops::apply(&mut n, &ops).map_err(RpcErr::app)?;
            let hash = lensbench_project::sysfile::system_hash(&n).map_err(RpcErr::app)?;
            let count = n.surfaces.len();
            store_changed(sess, n, spec, Change::Edit(ops));
            let mut v = history(sess);
            v["notes"] = json!(notes);
            v["surfaces"] = json!(count);
            v["system_hash"] = json!(hash);
            v
        }
        // 面のデータを全部の項目で（set_surface_data と同じ形）と、その面のソルブ・ピックアップ（GUI の面の設定）
        Request::SurfaceData { surface } => {
            let (s, _) = sess.current()?;
            let j = match surface {
                Ref::Name(t) => s.find_surface(&t)?,
                Ref::Index(j) => j,
            };
            lensbench_project::edit_ops::surface_data(&s, j).map_err(RpcErr::app)?
        }
        // 系の設定: 波長・視野・開口・エイミング・環境・GLRS・ROPD・膜ファイルなど（edit の set_system_data と同じ形）
        Request::SystemData => {
            let (s, _) = sess.current()?;
            let v = lensbench_project::edit_ops::system_data(&s).map_err(RpcErr::app)?;
            json!({ "data": v })
        }
        // 多重構成の表: 構成の数 n・今の構成 active・行（パラメータの名前と構成ごとの値）。
        // .zmx なら、ファイルの多重構成の数 zmx_cfgs と読んだ構成 zmx_cfg も
        Request::ConfigTable => {
            return Ok((
                Answer::ConfigTable(Box::new(config_table(sess)?)),
                Vec::new(),
            ))
        }
        Request::Configs => configs(sess)?,
        // 面の操作をプロジェクトのブロックへ書き戻した結果（保存しない、GUI の組合せの面の表）
        Request::WriteBack { ops } => save_project::write_back(sess, ops)?,
        // 系ファイルに保存する（編集・set の後の系をそのまま）
        Request::Save { path } => {
            let (s, _) = sess.current()?;
            lensbench_project::sysfile::save(&sess.files.check_path(&path, true)?, &s, &[])
                .map_err(RpcErr::app)?;
            let hash = lensbench_project::sysfile::system_hash(&s).map_err(RpcErr::app)?;
            let notes = lensbench_project::sysfile::save_notes(&s);
            json!({ "written": path, "system_hash": hash, "notes": notes })
        }
        // 今の系の面 from〜to だけを系ファイル（面の並び）に書き出す（GUI のレンズデータの「選んだ範囲を書き出す」）。
        // 物体面・像面は残し、範囲の外の面は消す（外を参照するピックアップ・ソルブは外れる）。絞りが範囲の外なら from を絞りに
        Request::SaveRange { path, from, to } => {
            let (s, _) = sess.current()?;
            let s = range_system(&s, from, to).map_err(|e| RpcErr::app(e.to_string()))?;
            lensbench_project::sysfile::save(&sess.files.check_path(&path, true)?, &s, &[])
                .map_err(RpcErr::app)?;
            let notes = lensbench_project::sysfile::save_notes(&s);
            json!({ "written": path, "surfaces": s.surfaces.len(), "notes": notes })
        }
        Request::Surfaces => {
            return Ok((Answer::Surfaces(Box::new(surfaces_view(sess)?)), Vec::new()))
        }
        Request::Analyze(r) => return Ok((analyze(sess, &r)?, Vec::new())),
        Request::Layout(r) => layout(sess, &r)?,
        // レイアウトをファイルに書き出す（GUI の書き出し。形式は拡張子で、CLI の layout -o と同じ）
        Request::LayoutExport(r) => layout_export(sess, &r)?,
        Request::Preview { load, layout } => preview_of(sess, &load, &layout)?,
        Request::Trace(r) => return trace(sess, &r, bins).map(|(v, b)| (Answer::Json(v), b)),
        Request::Plugin(r) => return plugin(sess, &r, bins).map(|(v, b)| (Answer::Json(v), b)),
        Request::Ping => json!({ "version": lensbench_core::VERSION }),
        Request::Describe => {
            let a: Vec<&str> = crate::analyses().iter().map(|d| d.meta().name).collect();
            let v = lensbench_core::VERSION;
            json!({ "version": v, "protocol": PROTOCOL, "methods": METHODS, "analyses": a })
        }
        Request::Shutdown => json!(true),
        // 1 つの面の値（番号は名前と同じく引く）
        Request::Surface { surface } => {
            let (s, _) = sess.current()?;
            let j = match surface {
                Ref::Name(t) => s.find_surface(&t)?,
                Ref::Index(j) => s.find_surface(&j.to_string())?,
            };
            let fields =
                lensbench_project::edit_ops::surf_fields_json(&s, j).map_err(RpcErr::app)?;
            json!({ "surface": j, "fields": fields })
        }
        // .zmx の多重構成を表にする edit の操作: 各構成を読み、構成で違うパラメータを行にする。
        // 表にできない違い（硝材・無視・面の数・半径以外の値）は notes に
        Request::ConfigDiff => {
            let Current { spec, src, .. } = sess.current_full()?;
            config_diff(&spec, &src)?
        }
        // 系ファイルに保存する（編集・set の後の系をそのまま）
        Request::SaveProject { path } => save_project::save_project(sess, &path)?,
        Request::Plugins => crate::load::plugins_json(),
        // analyze の解析の設定の項目（名前・型・既定・説明。GUI の設定欄）
        Request::AnalysisArgs { cmd } => {
            let args = crate::params::analysis_args(&cmd).ok_or_else(|| {
                RpcErr::params(format!(
                    "{cmd} is not an analyze analysis (see plugins for the list)"
                ))
            })?;
            json!({ "cmd": cmd, "args": args })
        }
        // カタログを探す一覧（GUI のカタログブラウザ）
        Request::Glasses(r) => catalog::glasses(sess, &r)?,
        Request::Lenses(r) => catalog::lenses(sess, &r)?,
        Request::CatalogSearch(r) => catalog::catalog_search(sess, &r)?,
        // ユーザーカタログ: 読み込んだプロジェクトのブロック・範囲を項目に保存、一覧、名前・説明の変更と削除
        Request::CatalogAdd(r) => catalog::catalog_add(sess, &r)?,
        Request::CatalogItems { catalog } => catalog::catalog_items(sess, &catalog)?,
        Request::CatalogEdit(r) => catalog::catalog_edit(sess, &r)?,
        // カタログの未完の品に厚さ・径を足し、補足のカタログ（ユーザーカタログ）に普通の品として保存
        Request::CatalogFill(r) => catalog::catalog_fill(sess, &r)?,
        // 品の詳しい形の STEP を 1 件取ってキャッシュに置き、簡易外形と比べる（CLI の cad-step）
        Request::CadStep(r) => catalog::cad_step(sess, &r)?,
    };
    Ok((Answer::Json(v), Vec::new()))
}

/// 多重構成の表（`config_table`）
fn config_table(sess: &Session) -> Result<views::ConfigTable, RpcErr> {
    let Current {
        sys: s, spec, src, ..
    } = sess.current_full()?;
    let rows: Vec<views::ConfigRow> = s
        .configs()
        .map(|c| {
            c.rows
                .iter()
                .map(|r| views::ConfigRow {
                    param: s.param_name(r.param),
                    values: r.values.clone(),
                    picks: r.picks.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let (n, active) = s.configs().map_or((1, 1), |c| (c.n, c.active));
    // 読み込んだときのファイルの中身から（.zmx でなければ 1）
    let zmx_cfgs = src.configurations()?;
    Ok(views::ConfigTable {
        n,
        active,
        rows,
        zmx_cfgs,
        zmx_cfg: spec.cfg,
    })
}

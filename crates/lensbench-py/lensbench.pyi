# 自動生成（crates/lensbench-py/tools/gen_stubs.py）。手で直さない
"""lensbench: 順次光線追跡（Python バインディング）"""

import os
from collections.abc import Callable, Mapping, Sequence
from typing import Any, final

import numpy as np
from numpy.typing import ArrayLike, NDArray

_StrPath = str | os.PathLike[str]
#: 視野: 番号（1 始まり）か正規化視野 (hx, hy)
_Field = int | tuple[float, float]
#: 面: 番号（OpticStudio の面番号）か名前（コメント）
_Surf = int | str
#: 物点: 物体面の (x, y) [mm]、無限遠なら角度 (θx, θy) [deg]
_ObjectPoint = tuple[float, float] | Sequence[float]

__version__: str
#: Python API の版（互換を壊す変更で上がる。docs/python.md）
API_VERSION: tuple[int, int]


@final
class Grid:
    """掃引の格子: {"3.t": [10, 11, 12], "5.dx": [0, 0.01]} の直積（値そのもの）。番号は最初の軸が最も遅く変わる順"""

    def __new__(cls, system: System, axes: Any) -> Grid: ...

    def __len__(self) -> int:
        """Return len(self)."""

    def draw(self, i: int) -> dict[str, Any]:
        """i 番目の組の値: {名前: 値}"""

    def system(self, i: int) -> System:
        """i 番目の組の値を入れた系"""


@final
class Plugin:
    """設定の決まったプラグイン。`Plugin(name, **config)`。`run(**inputs)` → dict、
    `run_many([inputs, ...], threads=None)` → [dict, ...]
    """

    def __new__(cls, name: str, **config: Any) -> Plugin: ...

    @property
    def config(self) -> dict[str, Any]:
        """設定（dict）"""

    @property
    def kind(self) -> str: ...

    @property
    def name(self) -> str: ...

    def run(self, system: System | None = None, **inputs: Any) -> dict[str, Any]:
        """1 回呼ぶ → 名前つきの配列の dict（Rust のプラグインは GIL を外して計算する）。`system` は系を追跡する
        プラグインに渡す `lb.System`
        """

    def run_many(self, inputs: Any, system: System | None = None, threads: int | None = None) -> list[Any]:
        """多数の入力（dict の並び）をまとめて呼ぶ → [dict, ...]。入力ごとに `threads` 本で並列（None は既定）、
        結果は入力の順でスレッド数によらない。どれかが誤りなら最初の誤りを出す
        """


@final
class Sampler:
    """抽選（モンテカルロ）: 系 system の公称値に、パラメータごとの分布のずれを足す。

    - params: {"3.t": 0.02, "5.dx": ("uniform", -0.01, 0.01), "G.tx": ("trunc", 0.05, 2.0)}（数だけなら正規分布の σ）
    - corr: 相関行列（params の順）。分布ごとの周辺分布はそのままで、ガウスのコピュラで相関を入れる
    - seed: 乱数の種。i 番目の組は (seed, i) だけで決まる（スレッド数・再開・順序によらない）
    """

    def __new__(cls, system: System, params: Any, corr: Sequence[Sequence[float]] | None = None, seed: int = 0) -> Sampler: ...

    def draw(self, i: int) -> dict[str, Any]:
        """i 番目の組の値（公称値 + ずれ）: {名前: 値}"""

    @property
    def seed(self) -> int: ...

    def system(self, i: int) -> System:
        """i 番目の組の値を入れた系"""


@final
class System:
    """光学系（.zmx またはプロジェクトファイル .json から読む）"""

    @property
    def active_config(self) -> int:
        """今の系に入っている構成（1 始まり）"""

    def add_glass_catalogs(self, names: Sequence[str], dirs: Sequence[_StrPath] | None = None) -> list[str]:
        """ガラスのカタログ（AGF の名前、例 "SCHOTT"）を読んで系に足す。dirs は探す場所（省略で既定の場所）。
        見つからなかったカタログの名前を返す
        """

    def add_pickup(self, dst: _Surf, src: _Surf, what: str, scale: float = 1.0, offset: float = 0.0) -> None:
        """ピックアップを付ける: 面 dst の what = scale × 面 src の what + offset（同じ先・量のものは置き換える）"""

    def add_solve(self, surf: _Surf, kind: str = "marginal_height", value: float = 0.0, other: _Surf | None = None, param: float = 0.0) -> None:
        """面 surf にソルブを付ける（同じ面・同じ量のものは置き換える）。kind は厚さの marginal_height（既定）・
        marginal_height_zone・chief_height・pupil_position・edge・opd・position・compensator・center_of_curvature、
        曲率の marginal_angle・chief_angle・marginal_normal・chief_normal・aplanatic・element_power・
        concentric_surface・concentric_radius・f_number。value はその値（高さ・角度・長さなど）、param は
        marginal_height_zone・opd の瞳の帯・edge の高さ、other は別の面を参照する種類の面
        """

    @property
    def aiming(self) -> str:
        """光線のエイミング（"off"・"paraxial"・"real"）"""

    def cardinal(self, wave: int | None = None) -> dict[str, Any]:
        """基点（OpticStudio の Cardinal Points 相当）。wave は波長の番号（1 始まり、省略で主波長）。

        面 1 から像面の手前の面までの近軸の系（Y-Z 面）。物体空間の位置は面 1 の頂点から、像空間の位置は像面の
        頂点から、それぞれの局所 z で測る [mm]（OpticStudio と同じ。鏡が奇数枚なら n_image と f_image は負）。
        `efl` は空気換算（EFFL と同じ）、`f_object`・`f_image` は媒質の中の焦点距離（物体空間は負）。
        `bfl` は最後の面から、`ffl` は面 1 から焦点までの距離。
        無焦点の系では inf。`set_param` などの変更後の系の値を返す。
        """

    def config(self, k: int) -> System:
        """構成 k（1 始まり）の値を入れた新しい系（元の系は変えない）"""

    @property
    def config_count(self) -> int:
        """多重構成の数（表が無ければ 1）"""

    def configs(self) -> list[tuple[str, list[float]]]:
        """多重構成の表: (パラメータ名, 構成ごとの値) の list"""

    def define_unit(self, name: str, spans: Sequence[tuple[int, int]], pivot: tuple[float, float, float] = ...) -> None:
        """玉・群を定義する。spans は面の範囲 (最初, 最後) の並び（ダブルパスの往きと戻りなど、同じ玉を 2 回通るなら
        2 つ）、pivot は回転の中心（最初の範囲の最初の面の頂点が原点の座標）。以後 "名前.dx" などで変えられる
        """

    def delete_surface(self, s: _Surf) -> list[str]:
        """面 s を消す。消した面を参照していたピックアップ・ソルブ・多重構成の行の説明の list を返す（警告にも出す）"""

    def dispersion_warnings(self) -> list[str]:
        """分散式の有効範囲（AGF の LD）の外の波長で屈折率を求めている材料の警告（式を延ばして計算している）"""

    def fiber_coupling(self, na_source: float, na_receiver: float | None = None, wave: int | None = None, source: tuple[float, float] = ..., receiver: tuple[float, float] = ..., grid: int = 101, na_max: float | None = None, pol: bool = False, clip_stop: bool = True) -> dict[str, Any]:
        """単一モードファイバへの結合効率（OpticStudio の FICL 相当）。送り側は物点 source（物体面の局所 [mm]）、
        受け側は像面の局所の点 receiver、どちらも遠視野の 1/e² の NA で決まるガウスモード（na_receiver を省くと
        na_source と同じ）。物体空間の sin θ ≤ na_max（省略で min(3·NA/n, 0.999)）を grid × grid の方向で標本化し、
        受け側の方向空間で重なり積分をとる。返す dict: eta（結合効率）・system（受け側まで届いた送り側の電力の比）・
        receiver（eta/system）。pol=True なら膜込みの偏光の重なり jones（(2, 2) complex、[出射 x/y, 入射 x/y]）も返し、
        入射偏光 e の結合は |jones @ e|²。clip_stop=True（既定）なら系の絞り（絞り面に開口がなければ絞り半径の円）の外を
        通る光を系の損失に数える（OpticStudio の FICL と同じ）。有限物体の系だけ
        """

    def field_mtf(self, freqs: ArrayLike, h: ArrayLike, axis: str = "y", wave: int | None = None, n: int | None = None, threads: int | None = None) -> dict[str, Any]:
        """フィールド MTF: 正規化視野 h（0〜1、axis "y" か "x" の向き）ごとの回折 MTF。
        tangential・sagittal は (周波数, h) の配列、field_value は視野の値（系の単位）
        """

    @property
    def field_type(self) -> Any:
        """視野の種類: "angle"・"object_height"・"paraxial_image_height"・"real_image_height"、
        条件で定める視野なら ("solve", ("dirsum", 面1, 面2)) など
        """

    @property
    def fields(self) -> list[tuple[float, float]]:
        """視野 (x, y)"""

    def find_surface(self, target: str) -> int:
        """面を番号か名前（コメント）で探し、面番号を返す"""

    @staticmethod
    def from_json(text: str) -> System:
        """系ファイルの JSON の文字列から系を作る（coatings のパスはそのまま使う）"""

    @staticmethod
    def from_project(project: Any, base: _StrPath | None = None, glass_dirs: Sequence[_StrPath] | None = None, state: str | None = None, lens_catalogs: Sequence[_StrPath] | None = None, coatings: Sequence[_StrPath] | None = None, coating_file: _StrPath | None = None, split_path: str | None = None) -> System:
        """プロジェクト（ブロックで組む系）を、ファイルを介さずに JSON の文字列か dict から展開する。

        - project: プロジェクトファイルと同じ形の JSON の文字列、または dict
        - base: 相対パス（zmx ブロックの path など）の基準のフォルダ（省略でカレント）
        - そのほかは load と同じ。取り込んだ .zmx のピックアップのうち展開で残せないもの（範囲の外を参照するなど）は
          値で固定し、warnings で知らせる
        """

    def geometric_mtf(self, field: _Field, freqs: ArrayLike, wave: int | None = None, n: int = 128, diff_limit: bool = False, threads: int | None = None) -> dict[str, Any]:
        """幾何 MTF: 光線の位置の分布から。n は瞳の求積の点の数の目安、diff_limit なら回折限界の MTF をかける。
        返す dict は mtf と同じ形
        """

    def get_param(self, spec: str) -> float:
        """パラメータの今の値。spec は "面.種類"（例 "3.t"、"L1 front.c"）。種類は c r k a2〜a16 t dx dy tx ty tz dn disp。
        玉・群は "名前.種類"（例 "lens2.tx"、"G1.shift"）で、種類は dx dy tx ty tz shift dn disp
        """

    def ghost(self, a: _Surf, b: _Surf) -> System:
        """ゴースト: 面 a で反射し、面 b で反射する（b < a）経路を順次の面に展開した系（膜の係数・偏光追跡もそのまま使える）。
        面は番号かコメント。展開した系の面ごとの元の面は ghost_origin で
        """

    def ghost_origin(self, a: _Surf, b: _Surf) -> Any:
        """ゴーストの系（ghost(a, b)）の面ごとの元の面の番号（挟んだ座標ブレークは None）と、反射の面の番号 (a, b)"""

    def ghost_pairs(self, first: int = 1, last: int | None = None) -> list[tuple[int, int]]:
        """2 回反射の組 [(a, b), ...]（a > b、a が先に反射）。first..=last の面（省略で 1〜像面。像面は膜があるときだけ）"""

    @property
    def hash(self) -> str:
        """系の hash（"sha256:…"、系ファイルと同じ）。解析の結果に刻んで、後から同じ系か確かめる"""

    def huygens_mtf(self, field: _Field, freqs: ArrayLike, wave: int | None = None, pupil: int = 64, image: int = 128, delta: float | None = None, threads: int | None = None) -> dict[str, Any]:
        """Huygens MTF: Huygens PSF のフーリエ変換（像面の上の周波数）。返す dict は mtf と同じ形"""

    def huygens_psf(self, field: _Field, wave: int | None = None, pupil: int = 64, image: int = 128, delta: float | None = None, threads: int | None = None) -> dict[str, Any]:
        """Huygens PSF: 射出瞳の参照球の各点からの 2 次波を像面の上で足す（傾いた像面・収差の大きい系向け）。
        返す dict は psf と同じ形で、x・y は中心 center からの像面の局所座標の差（axes は x̂・ŷ）
        """

    def hxhy(self, i: int) -> tuple[float, float]:
        """視野番号 i（1 始まり）の正規化視野 (hx, hy)"""

    @property
    def image(self) -> int:
        """像面の面番号"""

    @property
    def infinite(self) -> bool:
        """無限遠物体か"""

    def insert_surface(self, at: int, **kw: Any) -> None:
        """面 at の前に面を入れる（at は 1〜像面）。値はキーワードで（surface() の名前。type material r t など）。
        入れた面の後ろの媒質はその面の材料（省略で空気）
        """

    @staticmethod
    def load(path: _StrPath, glass_dirs: Sequence[_StrPath] | None = None, cfg: int = 1, state: str | None = None, lens_catalogs: Sequence[_StrPath] | None = None, coatings: Sequence[_StrPath] | None = None, coating_file: _StrPath | None = None, wavelengths: Sequence[float] | None = None, split_path: str | None = None) -> System:
        """系を読む。path は .zmx かプロジェクトファイル（.json）。

        - glass_dirs: ガラスカタログ（AGF）を探すフォルダ
        - cfg: .zmx のマルチコンフィグの番号（1 始まり）
        - state: プロジェクトファイルの状態の名前
        - split_path: プロジェクトファイルのビームスプリッターの経路（BS を通る順の T・R、例 "TR"）。
          とれる経路は `System.split_paths(path)`
        - lens_catalogs: プロジェクトファイルのカタログレンズを探すレンズカタログ（.zmx またはフォルダ）
        - coatings: 偏光追跡で使う膜の実測表（LBCT）・モデル（LBCM）のファイル
        - coating_file: 系の膜ファイル（COATING.DAT 形式）を置き換える
        - wavelengths: 波長 [µm] を置き換える（.zmx だけ。重みは 1、主波長の番号はそのまま）
        """

    @staticmethod
    def load_configs(path: _StrPath, configs: Sequence[int] | None = None, glass_dirs: Sequence[_StrPath] | None = None, coatings: Sequence[_StrPath] | None = None, coating_file: _StrPath | None = None) -> list[System]:
        """多重構成の .zmx から複数の構成の系をまとめて読む。configs は構成の番号（1 始まり）の列で、
        省くと全部（空の list はエラー）。返すのは System の list で、それぞれ `load(path, cfg=k)` と同じ。
        試料腕・参照腕や絞りの径の切替（APMX・APER など）を、`trace_many(..., object=物点)` で同じ物点・
        同じ瞳の格子でまとめて追跡するのに使う
        """

    def macro_warnings(self) -> list[str]:
        """ZPL Macro Solve のうち、解かずに保存された値で固定したもの（マクロがない・対応していない命令など）の理由"""

    def mech_check(self, fields: Sequence[int] | None = None, waves: Sequence[int] | None = None, grid: int = 21) -> dict[str, Any]:
        """機構部品（プロジェクトの mechanics）のケラレ。CLI の `mech-check` と同じ。

        瞳の正方格子（一辺 grid 本）の単位円の中の光線を、視野 fields・波長 waves（どちらも 1 始まり、省略で全部）
        ごとに追い、部品の穴の外に出た割合 [%] を数える。戻り値は dict:
        `pupil_rays`（1 視野・波長あたりの光線数）、`parts`（部品ごとの name・surface・clipped_max_pct・
        first_max_pct・bores）、`results`（視野・波長ごとの field・wave・rays・failed・optical_pct・passed_pct・
        parts=[{name, clipped_pct, first_pct}]）。clipped は部品の穴の外に出た光線（先に切れたものも数える）、
        first は光の通る順で最初にその部品で切れた光線、optical は部品より先に系の開口で切れた光線。
        `fit` は配置の確認: kind（collision・protrude・loose・overlap・rod・rod_mismatch）・error・
        part・other・surface・value [mm]・message の dict の並び。
        """

    def merit_jacobian(self, vars: Sequence[str | tuple[str, float, float]], merit: str | None = ..., reference: str = "centroid", rings: int = 3, jac: str = "auto", threads: int | None = None, terms: Any = None) -> tuple[Any, Any]:
        """最適化の評価関数の残差 r とヤコビアン J（[残差][変数]）を、今の変数の値で返す（反復は自前で回す使い方。
        引数は optimize と同じ。terms の項も使える）。Σ r² / Σ 重み の平方根が評価関数の値。残差は項の順
        （既定の評価関数の項、terms の順）で、各項に重みの平方根を掛けたもの
        """

    def mtf(self, field: _Field, freqs: ArrayLike, wave: int | None = None, n: int | None = None, threads: int | None = None, method: str = "auto", each_wave: bool = False) -> dict[str, Any]:
        """回折 MTF（CLI の mtf と同じ）。field は番号（1 始まり）か正規化視野 (hx, hy)、wave は波長の番号（省略で
        多色）、n は瞳の重なりの求積の点の数の目安（省略で自動）。method は "auto"（既定。"interp" で、n を省いて
        求積の点数が 1024 を超え、周波数が 8 個以上なら "fft"）・"interp"（補間の求積）・"fft"（瞳関数の自己相関を FFT で全周波数まとめて。
        周波数が多いとき速い、求積との差 1e-4 程度）・"direct"（確かめ用）。
        返す dict: freqs・tangential・sagittal（numpy）・cutoff（x・y の遮断周波数 [cycles/mm]）。
        each_wave=True なら多色に加えて waves（波長ごとの dict のリスト。別々に求めるより速い）も返す（wave と一緒に使えない）
        """

    @property
    def name(self) -> str: ...

    def object_of(self, field: _Field) -> list[float]:
        """視野（番号か (hx, hy)）の物体: 有限物体なら物点 (x, y) [mm]、無限遠なら方向 (l, m, n)"""

    def optimize(self, vars: Sequence[str | tuple[str, float, float]], merit: str | None = ..., reference: str = "centroid", rings: int = 3, iters: int = 50, jac: str = "auto", threads: int | None = None, terms: Any = None) -> dict[str, Any]:
        """減衰最小二乗（DLS）で最適化した系を返す（元の系は変えない）。vars は変数の列で、各要素は "7.t" か
        ("7.t", 下限, 上限)（None で片側なし）。既定の評価関数 merit は "spot"（RMS スポット）か "wavefront"（RMS 波面）で、
        全視野（視野の重み）・多色。None なら terms の項だけ。jac は "auto"（解析的に求められる列が 16 個以上の項は解析的に、少ない項は中心差分）か
        "fd"（すべて中心差分）か "forward"（すべて前進差分、中心差分の半分の追跡）。

        termsは足す項の dict の並び。{"type": "ray", "field": 1, "py": 1, "q": "y", "target": 0, "weight": 1} の
        形で、種類は spot・wavefront・mtf・paraxial（q: efl・pmag・enpp・epd・expp・expd・wfno・pwfn・isfn・pimh・totr・
        obna・isna）・ray（q: x・y・z・l・m・n、面 surf の局所座標）・opd・zernike・coupling・zplm（ZPL マクロの OPTRETURN）・
        plugin（登録したプラグインの出力。CLI の --term と同じ）。
        {"type": "user", "fn": f, "jac": g, "name": ..., "weight": ...} は Python の関数で定義する項: f(systems) は
        System の list を受け取り、系ごとの残差を (len(systems), 残差の数) の配列で返す（差分の系はまとめて 1 回で渡す）。
        g(system, names) を与えると残差の偏微分 (残差の数, len(names)) をそれで求める。

        返り値の dict: system（最適化した系）・x0・x（変数の値）・merit0・merit・history（反復ごとの評価関数）・
        iterations・evals・stop（"converged"・"small_step"・"no_progress"・"max_iter"）・term_values0・term_values
        （terms の項ごとの値: 目標のある項は値、ほかは √Σr²）
        """

    def pickups(self) -> list[dict[str, Any]]:
        """ピックアップの一覧: dst・src・what（c t k p1〜）・scale・offset の dict"""

    @property
    def primary(self) -> int:
        """主波長の番号（1 始まり）"""

    def psf(self, field: _Field, wave: int | None = None, pupil: int = 64, image: int = 128, delta: float | None = None, threads: int | None = None) -> dict[str, Any]:
        """回折 PSF: 瞳関数（MTF と同じ）の DFT。強度 i は (y, x) の配列で、収差のない同じ瞳の多色 PSF の中心を
        1 とする（peak はストレール比の目安）。x・y は主波長の主光線の像点 center からの、主光線に垂直な面の座標
        [レンズ単位]（OpticStudio の FFT PSF と同じ。向きは axes = [e1, e2]、像空間の単位ベクトル）。
        pupil は瞳の格子の一辺、image は像の格子の一辺、delta は像の格子の間隔（省略で 1 / (2 × 遮断周波数)）
        """

    @property
    def pupil_reversed(self) -> bool:
        """有限物体で入射瞳が物体より前にあるか（pupil="normalized" の向きが "signed" と逆になる系）"""

    def remove_config_row(self, spec: str) -> None:
        """表の行を消す（系の値は今のまま）"""

    def remove_pickup(self, dst: _Surf, what: str) -> None:
        """面 dst の what のピックアップを外す（値は今のまま）"""

    def remove_solve(self, surf: _Surf, what: str = "t") -> None:
        """面 surf の what（"t" 厚さ / "c" 曲率）のソルブを外す（値は今のまま）"""

    def rms(self, field: _Field | None = None, vs: str | None = None, x: ArrayLike | None = None, wave: int | None = None, axis: str = "y", reference: str = "centroid", rings: int = 6, threads: int | None = None, field_wave: bool = False) -> dict[str, Any]:
        """RMS: スポット半径（spot・spot_x・spot_y [レンズ単位]）、波面収差 wfe [波]、ストレール比の近似
        strehl = exp(−(2π wfe)²)。横軸 vs は "field"（x は正規化視野、axis の向き）・"focus"（x は像面のずらし）・
        "wavelength"（x は波長 [µm]、単色）・None（1 点、field の値）。reference は "centroid" か "chief"、
        rings は瞳のガウス求積のリング数。field は vs が focus・wavelength・None のとき使う。field_wave=True で、
        単色の field・wavelength の掃引の実像高（と条件で定める視野）の視野を解析の波長の主光線で解く（OpticStudio と同じ）
        """

    def save(self, path: _StrPath) -> None:
        """系ファイル（lensbench 独自の JSON）に保存する。面・ガラスのデータ・パラメータの変更・ピックアップ・
        ソルブ・玉と群・多重構成の表・dn と disp を欠けなく書き、System.load で読み直すと追跡はビット一致する。
        膜の実測表・モデル（coatings=）のパスも書く
        """

    def set_config(self, k: int) -> None:
        """今の系を構成 k に切り替える"""

    def set_config_count(self, n: int) -> None:
        """構成の数を n にする（増やした構成の値は今の値、1 で表を消す）"""

    def set_config_row(self, spec: str, values: Sequence[float]) -> None:
        """表の行を足す（同じパラメータは置き換える）: spec は set_param と同じ名前、values は構成ごとの値"""

    def set_field_type(self, kind: str, cond: Any = None, fields: Sequence[tuple[float, float]] | None = None) -> None:
        """視野の種類と値を変える。kind は field_type と同じ名前。"solve" なら cond に条件
        （("position", 面)、("dirsum", 面1, 面2)、("dirdiff", 面1, 面2)、面は番号か名前）を与え、
        視野の (x, y) が条件の目標値になる物体を主波長の主光線で解く（パラメータを変えると解き直す）。
        fields を与えると視野の (x, y) も置き換える（重み・ケラレ係数は既定に戻る）
        """

    def set_material(self, s: _Surf, material: str) -> None:
        """面 s の材料: ガラス名（系のカタログ、add_glass_catalogs で足せる）・"nd,vd[,dpgf]"・"" か "AIR"・"MIRROR"
        """

    def set_param(self, spec: str, value: float) -> None:
        """パラメータを変える（系を作り直さない）。ピックアップ先は元の面に合わせて解き直す（ピックアップ先そのものは変えられない）"""

    def set_params(self, changes: Any) -> None:
        """まとめて変える: {"3.t": 10.0, "5.dx": 0.01} か [("3.t", 10.0), ...]"""

    def set_stop(self, s: _Surf) -> None:
        """面 s を絞りにする"""

    def set_surface(self, s: _Surf, **kw: Any) -> None:
        """面 s の値をキーワードでまとめて変える（surface() の名前。mirror・stop・n は除く。材料は material）"""

    def set_surface_type(self, s: _Surf, kind: str) -> None:
        """面 s の種類（STANDARD EVENASPH COORDBRK JONESMAT PARAXIAL TOROIDAL FRESNELS）。新しい種類に無い値は 0 に戻す"""

    def set_unit_pivot(self, name: str, pivot: tuple[float, float, float]) -> None:
        """玉・群の回転の中心を変える"""

    def solve_ray(self, unknown: str, residual: Any, object: _ObjectPoint | None = None, field: _Field | None = None, pupil: tuple[float, float] = ..., wavelength: int | None = None, tol: float = 1e-12, max_iter: int = 50) -> dict[str, Any]:
        """未知数 2 つ・残差 2 つの実光線を Newton 法で解く。

        - unknown: "object"（物点の x・y [mm]、有限物体）か "pupil"（瞳座標 px・py）
        - residual: ("position", 面, (x, y))（面での局所座標の位置 − 目標）、("dirsum", 面1, 面2)（方向余弦
          l・m の和）、("dirdiff", 面1, 面2)（差）のどれか
        - object: 物体（物点 (x, y) か方向 (l, m, n)）。未知数が物点なら初期値。省くと field の物体
        - field: object を省いたときの視野（番号か (hx, hy)、省くと軸上）
        - pupil: 瞳座標。未知数が瞳座標なら初期値
        - wavelength: 波長の番号（省くと主波長）

        返す dict: object、pupil、residual、iterations（そのまま trace(None, [px], [py], object=...) に渡せる）
        """

    def solves(self) -> list[dict[str, Any]]:
        """ソルブの一覧: surf・what（"t" 厚さ / "c" 曲率）・kind（種類の名前）・value・param・other の dict"""

    @staticmethod
    def split_paths(path: _StrPath, glass_dirs: Sequence[_StrPath] | None = None, state: str | None = None, lens_catalogs: Sequence[_StrPath] | None = None) -> list[str]:
        """プロジェクトファイル（.json）のビームスプリッターの経路の一覧（BS を通る順の T・R の文字列）。
        BS がなければ空。それぞれ `load(path, split_path=経路)` で読む
        """

    @property
    def stop(self) -> int:
        """絞りの面番号"""

    def surface(self, s: _Surf) -> dict[str, Any]:
        """面 s の値の dict（type comment t c r k asph cx material mirror coating semi stop ignore aperture cb cb_order
        focal jones dn disp n）。r は曲率半径（平面は inf）、aperture は dict か None
        """

    @property
    def surface_count(self) -> int:
        """面の数（物体面と像面を含む）"""

    def system_data(self) -> dict[str, Any]:
        """システムデータ（OpticStudio のオペランド名の小文字: effl・epdi・enpp・expd・expp・wfno・pwfn・isfn・
        pimh・pmag・totr・obna・isna）。主波長、変更後の系の値
        """

    def through_focus_mtf(self, field: _Field, freqs: ArrayLike, dz: ArrayLike, wave: int | None = None, n: int | None = None, threads: int | None = None) -> dict[str, Any]:
        """スルーフォーカス MTF: 像面を dz [レンズ単位] ずつずらした回折 MTF（像面の手前の厚さを変える。
        厚さのソルブは解き直さない）。tangential・sagittal は (周波数, dz) の配列。計算できない点は NaN で、
        理由は warnings
        """

    def to_json(self) -> str:
        """系ファイルの中身（JSON の文字列）"""

    def tolerance(self, spec: Any = None, terms: Any = None, vendors: Any = None, configs: Sequence[System] | None = None, threads: int | None = None) -> Any:
        """モンテカルロ公差シミュレーション。spec は CLI の `lensbench tolerance --spec` と同じ設定の dict
        （trials・seed・dist・preset・catalog・catalog_tols・lenses・groups・tols・compensators・comp_iters・merit・rings・
        ref・terms・sensitivity・percentiles・samples）。None なら既定（一般の公差の組を全部の玉に、RMS 波面、100 回）。
        terms は optimize の terms と同じ並びで、spec の terms の後に足す（Python の関数の項 {"type": "user", ...} も使える。
        測定値などを評価量にするときに）。vendors はブロックの id → メーカーの dict（カタログの玉の既定の公差に使う）。
        configs は追加の構成（この系が構成 1、ずれはすべての構成に同じく与える）。

        返り値の dict: nominal（merit・terms・comp）・merit（統計: n・mean・std・min・max・percentiles）・cdf（評価関数の
        昇順）・terms（項ごとの label・nominal・stats・cdf）・compensators・sensitivity（公差ごとの lo・hi での評価関数の
        変化、悪い順）・rss_estimate・correlation（ずれとの相関 r と |ずれ| との相関 r_abs）・tolerances・trials・failed・
        errors（samples が true なら試行ごとの deltas・eval も）
        """

    def trace(self, field: _Field | None, px: ArrayLike, py: ArrayLike, wavelengths: Sequence[int] | None = None, last: int | None = None, record: Any = ..., pol: Any = None, pol_phase: bool = False, globals: bool = False, object: _ObjectPoint | None = None, grad: Sequence[str] | None = None, threads: int | None = None, pupil: str = "normalized", dz: float | None = None, land: bool = False, pol_grid: Any = None) -> dict[str, Any]:
        """光線を追跡し、numpy 配列の dict を返す。

        - field: 視野の番号（1 始まり）か正規化視野 (hx, hy)。object を与えるときは None
        - px, py: 正規化瞳座標（1 次元、同じ長さ。list・float32 も可）
        - wavelengths: 波長の番号（1 始まり）の列。省くと全波長
        - last: この面で止める（省くと像面）
        - record: True なら途中の全面の交点も返す（`rec_*`、形は (波長, 面, 光線)）。面の番号の列なら
          その面だけ（無視する面・last より後は除く。面の多い系で速い）
        - pol: 偏光追跡する波長の番号（True なら主波長）。`jones` に (光線, 2, 2) の complex128 を返す。
          列は入射偏光（物体側の局所 x・y の単位ベクトル）、行は最後の面の局所座標の出射電場の x・y 成分。
          z 成分は `jones_z`（(光線, 2)）。膜の振幅・位相込み、位相の約束は E ∝ exp(−ikL)。止まった光線は NaN
        - pol_phase: True なら jones に伝搬の位相 exp(−ik·OPL) を掛ける（既定は掛けない。光路長は `opl`）
        - pol_grid: 偏光追跡を正規化瞳座標 [−1, 1]² の粗い格子（点数 n か (nx, ny)、各軸 4 点以上。例 25）で行い、
          伝搬の位相を除いた Jones 行列を 2 次元の 3 次スプライン（not-a-knot）で全光線へ補間する（全光線の偏光
          追跡より速い）。止まった・けられた格子点は格子で最も近い有効な点で埋める。瞳の内側（px² + py² ≤ 1）で欠けた
          格子点は `pol_grid_missing`（(点, 2) の px・py）に返し、そこから 2 点の範囲のセルにある光線は jones を NaN に
          して `pol_grid_bad`（(光線,) の bool）を True にする。pupil が "normalized"・"signed" のときだけ
        - globals: True なら大域の位置・方向 gx, gy, gz, gl, gm, gn と開始点・方向 p0, k0（(波長, 光線, 3)）も返す
        - object: 視野の代わりに物体を直接: 有限物体なら物点 (x, y) [mm]、無限遠物体なら方向 (l, m, n)。
          ケラレ係数はかけない
        - grad: 光路長を微分するパラメータ（"3.t"、"objx" など）の列。`grad` に dOPL/dp（(波長, 光線, パラメータ)）を返す。
          始点と最後の面の交点（局所 x・y）を固定した微分。objx・objy は有限物体のみ
        - land: True なら grad と一緒に `grad_land`（(波長, 光線, パラメータ, 4)）に最後の面での着地点・方向の微分
          d(x, y, l, m)/dp を返す。開始位置と開始方向を固定した微分（同じ光線を打ち出して系だけ変える。
          pupil="stop" や照準した光線でも、解き直さない）
        - dz: 検出面を最後の面の局所座標の z = dz の平面に置く（面の傾き・偏心に従う平面）。x, y, z・opl・
          opl_os・大域の位置をその平面での値にし、grad はその平面の x・y を固定した微分、grad_land もその平面で。
          記録（record）は変えない。pol_phase=True とは一緒に使えない
        - threads: スレッド数（1 で逐次。省くと全コア）。結果はスレッド数によらず同じビット
        - pupil: "normalized"（既定）なら px・py は正規化瞳座標（OpticStudio と同じ。入射瞳が物体より前の
          有限物体系では px = 1 が絞りの −x 側を通り、テレセントリックをまたぐと向きが反転する）。"signed" は
          その系で符号を反転した瞳座標（px = 1 が常に絞りの同じ側。`System.pupil_reversed` が True の系で
          normalized と符号が逆）。"stop" なら絞り面の局所座標 [mm] で、
          その点を通る実光線を光線ごとに解く（系のエイミング・入射瞳の位置によらない。ケラレ係数はかけない。
          解けない光線は err が絞りの面）

        返す配列（形は (波長, 光線)）: x, y, z, l, m, n（最後の面の局所座標の位置・方向余弦）,
        opl（光路長）, opl_os（OpticStudio の生の光路長）, lens_opd（理想レンズの位相）, err・vig（止まった・
        けられた面の番号、0 は通過）, ok（err も vig も 0）。ほかに wavelengths（番号）, last、視野なら hx・hy、
        object なら object。record のとき rec_surfaces と rec_x … rec_opl（rec_gx・rec_gy・rec_gz は大域）
        """

    def trace_rays(self, start: ArrayLike, direction: ArrayLike, wavelengths: Sequence[int] | None = None, last: int | None = None, record: Any = ..., pol: Any = None, pol_phase: bool = False, globals: bool = False, frame: str = "object", grad: Sequence[str] | None = None, threads: int | None = None, dz: float | None = None, land: bool = False) -> dict[str, Any]:
        """開始位置と方向を直接与えて追跡する。返す dict は [`trace`] と同じ（hx・hy・object はない）。

        - start, direction: (光線, 3) の配列。方向は規格化する（0 ベクトルは止まった光線）。光路長は開始位置から
        - frame: "object"（既定。有限物体なら物体面の局所座標、無限遠物体なら面 1 の変換の前の座標系）か "global"（大域）
        - ほかの引数は trace と同じ。grad の objx・objy は開始位置を物体面と一緒に動かした微分
        """

    def units(self) -> list[dict[str, Any]]:
        """玉・群の一覧: name・spans（面の範囲 [最初, 最後] の並び）・pivot・motion（dx dy tx ty tz）・shift・auto の dict"""

    @property
    def wavelengths(self) -> list[float]:
        """波長 [µm]"""

    @property
    def weights(self) -> list[float]: ...

    def with_params(self, changes: Any) -> System:
        """変えた系を新しく返す（元の系は変えない。軽い複製）"""


def _system_from_state(state: Any, coatings: Sequence[_StrPath]) -> System:
    """pickle から系を作り直す（[`PySystem::__reduce__`] の対）"""


def fringe_detector(sample: System, reference: System, field: _Field = ..., wave: int | None = None, pupil: int = 65, npx: int = 256, center: tuple[float, float] | None = None, width: float | None = None, sample_power: float = 1.0, reference_power: float = 1.0, delay: float = 0.0, polarized: bool = False, sample_input: str = "x", reference_input: str = "x", threads: int | None = None) -> dict[str, Any]:
    """像面上の干渉縞: 試料（sample）と参照（reference）の 2 本の経路の光線の格子（瞳 pupil²、一様に照らす）を像面の
    画素（npx²、中心 center = (x, y)・一辺 width [mm]、省略で 2 つが届く範囲）へ補間し、場を足す。
    polarized なら偏光追跡の場（入射偏光 sample_input・reference_input、膜は試料の系の膜）で重ねる。
    返す dict: x・y（画素の中心 [mm]）、intensity・i_sample・i_reference（(npx, npx)、行 = y）、opd [波]・
    visibility（両方届く画素だけ、ほかは NaN）、ok_sample・ok_reference、wavelength [µm]、fold（折り返しの割合）
    """


def interferogram(beam1: System, beam2: System | None = None, field: _Field = ..., wave: int | None = None, n: int = 64, scale: float = 1.0, xtilt: float = 0.0, ytilt: float = 0.0, threads: int | None = None) -> dict[str, Any]:
    """瞳上の干渉縞（OpticStudio の Interferogram に当たる）: 光束 1（beam1）と光束 2（beam2、None なら参照球）の
    波面（それぞれの主光線・参照球に対する OPD [波]）を同じ正規化瞳座標で引き、
    Δ = scale·(W1 − W2) + (xtilt·px + ytilt·py)/2（傾きは瞳の直径あたりの縞の数）、強度 (1 + cos 2πΔ)/2。多重構成なら `System.config(k)` の系を渡す。
    格子は OpticStudio の Interferogram と同じ（p_i = (i − n/2 + 1)/(n/2 − 1)、瞳は入射瞳の正規化座標 = OS の
    Use Exit Pupil を外した設定）。返す dict: p（格子の正規化瞳座標）、delta・intensity（(n, n)、行 = py、外は NaN）、
    pv・rms（scale·(W1 − W2) の値 [波]、傾きを除く）
    """


def plugin_spec(name: str) -> dict[str, Any]:
    """プラグインの説明 → {"name", "kind", "doc", "config", "inputs", "outputs"}"""


def plugins() -> list[tuple[str, str]]:
    """登録されたプラグインの一覧 → [(名前, 種類), ...]"""


def progress(done: int, total: int, message: str = "") -> None:
    """進み具合を 1 行で標準エラーに出す: `lensbench:progress 3/10 メッセージ`。
    `lensbench run` はこの行を拾ってログに残し、将来の GUI は進捗の表示に使う
    """


def read_batch(path: _StrPath) -> tuple[Any, list[Any]]:
    """一括実行の結果のファイル（JSONL）を読む: (頭の dict, [{"i", "seed", "result"}] 番号の順)"""


def register_plugin(name: str, func: Callable[..., Mapping[str, Any]], kind: str = "analysis", needs_system: bool = False) -> None:
    """Python の関数をプラグインとして登録する（同じ名前は差し替える）。
    `func(config: dict, inputs: dict[str, ndarray]) -> dict[str, array]`。配列は呼び出しごとにまとめて渡す。
    系を渡して呼ぶと `func(config, inputs, system)`。`needs_system=True` なら系の膜も読んでから渡す
    """


def run_batch(func: Callable[..., Any], plan: Any, n: int | None = None, out: _StrPath | None = None, threads: int | None = None, resume: bool = True, progress: Callable[[int, int], Any] | None = None, header: Any = None) -> list[Any]:
    """一括実行: plan の各組で func(i, system, seed) を呼ぶ。func に拡張の Rust の模擬（`crate::ext::NativeTask`）を
    渡すと、組の系で Rust のまま（GIL なしで）回す。

    - plan: `Sampler` か `Grid`。system は i 番目の組の値を入れた系、seed はタスクごとの種（雑音などに使う）
    - n: 組の数（Grid は省くと全部）
    - out: 結果を書く JSONL（1 行目に版・系の hash・種・組の作り方、以後 1 行 1 組で終わった順）。止まっても
      resume=True なら同じファイルの続きから（頭が違えばエラー）。None なら書かない
    - threads: スレッド数（1 で逐次）。progress: 省略か True で標準エラーに進み具合と残り時間（1 秒ごと）、False で出さない、
      関数なら dict（done total ran elapsed eta）で呼ぶ
    - header: 頭に足す値（JSON にできる dict など。再開のときは同じに）

    返すのは番号の順の結果の list（out があればファイルから読み直したもの）。func が例外を出すと新しい組を始めず、
    走っている組が終わってから同じ例外を出す（書いた結果は残るので再開できる）。Ctrl-C でも同じ
    """


def run_settings() -> dict[str, Any]:
    """`lensbench run`が渡す設定。どれも環境変数から読み、無ければ空か None

    - glass_dirs: ガラスカタログを探すフォルダ（LENSBENCH_GLASSCAT。System.load などは自分でも読む）
    - coatings: 膜の実測表・モデル（LENSBENCH_COATINGS。System.load の coatings= の既定）
    - threads: スレッド数（LENSBENCH_THREADS。threads= を省いた計算の既定）
    - out: 出力先のフォルダ（LENSBENCH_OUT）。スクリプトが自分で使う
    - log: ログのファイル（LENSBENCH_LOG）
    - args: スクリプトに渡した引数（sys.argv[1:] と同じ）
    - under_run: `lensbench run` から起動されたか
    """


def swap_search(project: _StrPath, spec: Any, glass_dirs: Sequence[_StrPath] | None = None, lens_catalogs: Sequence[_StrPath] | None = None, top: int = 0, threads: int | None = None, out: _StrPath | None = None, resume: bool = True) -> Any:
    """カタログレンズの差し替え探索: project（プロジェクトファイルのパス）の玉のブロックを spec（dict か JSON の
    文字列。CLI の swap --spec と同じ: slots・vars・merit・iters・efl・state など）の候補で差し替えた組を並列に回し、
    組ごとに変数を最適化する。返り値は組の結果の dict の list（評価関数の小さい順、top > 0 なら先頭の top 個）。
    各 dict は CLI の swap --json の results と同じ（i・lenses・merit0・merit・image_merit・vars・efl・stop・
    iterations・total_track、評価できなければ error）。out（JSONL）を与えると組ごとに逐次書き出し、resume なら
    同じ探索のファイルの続きから回す（一括実行の形）
    """


def trace_many(systems: Sequence[System], field: _Field | None, px: ArrayLike, py: ArrayLike, wavelengths: Sequence[int] | None = None, last: int | None = None, record: bool | Sequence[int] = ..., pol: bool | int | None = None, pol_phase: bool = False, globals: bool = False, object: _ObjectPoint | None = None, grad: Sequence[str] | None = None, threads: int | None = None, pupil: str = "normalized", dz: float | ArrayLike | None = None, land: bool = False) -> list[dict[str, Any]]:
    """多数の系を 1 回の呼び出しでまとめて追跡する。返すのは系ごとの dict の list で、
    それぞれ `systems[i].trace(field, px, py, ...)` と同じビット（スレッド数にもよらない）。

    引数は System.trace と同じで、全部の系に同じものを使う（grad のパラメータ名も系ごとに解く）。
    系の単位と光線の単位の両方で並列にするので、系ごとの光線が少なくてもコアを使い切る。
    threads は全体のスレッド数（1 で逐次）
    """


def unregister_plugin(name: str) -> bool:
    """登録を外す（あれば True）"""


def verify(paths: Any, ci: bool = False) -> Any:
    """照合: 記述（*.verify.json）かそれを含むフォルダ（1 つか並び）を、登録のプラグイン（同梱・`register_plugin` で
    足した分）で回し、照合ごとの結果の dict の並びを返す（`lensbench verify --json` の reports と同じ形）。
    ci=True で ci: false の照合を飛ばす
    """


def verify_compare(actual: Mapping[str, Any], expected: Mapping[str, Any], checks: Mapping[str, Any]) -> Any:
    """照合の比べ方だけを使う: 出力 actual と基準 expected（名前 → 配列）を、checks（名前 → {abs, rel, exact,
    note}、記述の checks と同じ）で比べ、量ごとの結果の dict の並びを返す。基準の名前には全部 checks が要る
    """


def zoom(systems: Sequence[System], vars: Sequence[str | tuple[str, float, float]], efl: Sequence[float | None] | None = None, pmag: Sequence[float | None] | None = None, steps: int = 0, merit: str = "spot", reference: str = "centroid", rings: int = 3, iters: int = 50, tol: float = 1e-8, threads: int | None = None) -> dict[str, Any]:
    """ズーム軌跡: 状態（systems、ズーム位置ごとの系。同じ形の系）ごとに vars（"G2.shift" か
    ("G2.shift", 下限, 上限)）を解く。評価関数は像の RMS（merit: "spot"・"wavefront"、全視野・多色）と、
    efl か pmag の目標（状態ごとの値の list、None は目標なし）。steps は隣り合う状態の間にとる点の数（継続法）。
    返り値の dict: quantity（"efl"・"pmag"）・names（変数の名前）・path（点の dict の list: pos・state（状態の番号、
    間の点は None）・target・value（近軸量）・x・merit（像だけの評価関数）・stop・iterations）・systems（状態ごとの
    解いた系）・warnings
    """

# HayateOffice 設計ドキュメント

Rust製の軽量・高速Officeスイート。既存（LibreOffice / ONLYOFFICE）の不満を設計原則で解決する。
本ドキュメントは設計議論の生きた記録であり、決定が進むたびに追記・更新する。

最終更新: 2026-06-02

---

## 1. プロジェクトの目的

既存Officeの4つの不満を、そのまま設計原則に落として解決する。

| 既存の不満 | HayateOfficeの設計原則 |
|---|---|
| 全部入り（Excel/PPT等を一括インストール） | **モジュラー**（必要なモジュールだけ） |
| 機能過多でUXが悪い | **段階的開示**・コマンドパレット中心 |
| 遅い | **Rust + GPU描画 + 仮想化** |
| 互換性で抽象化が弱い | **独自IR + 変換プラグイン + passthrough** |

## 2. 確定した技術選定

- **MVP = プレゼンテーションエディタ**（表計算/ワープロより先に1本を深く作る）
- **UI = gpui**（Zed製、GPU描画。Linux は Blade/Vulkan backend）
  - エントリは別クレート `gpui_platform::application()`、API は `gpui`。**両方必要**
- **フォーマット = 独自IR + 変換プラグイン**

## 3. gpui の入手方法（確定）

- crates.io 0.2.2 は古いので**使わない**。**`jijinbei/zed`（Zedのfork）をgit依存**で使う
- 検証済みコミット **`4fe71ffcc6a01b007df7e035df23e80647143b31`** にピン留め（fork済み・上流と同一HEAD）
- 理由: ①壊れた上流を使わされない（自分の検証済み点に凍結できる）②直したい時に自分のコピーを直せる
- gpuiを直接いじる時は fork をローカルclone（`crates/gpui` のスパースチェックアウト可）し、
  `[patch."https://github.com/jijinbei/zed"]` でパス差し替え（編集ループ最速化）
- ⚠️ Linuxビルドに `libwayland-dev` / `vulkan-tools` の追加が要りそう
  （`libvulkan1`/`libxkbcommon-dev`/`libxcb1-dev`/`libfontconfig1-dev` は導入済み、GPUは RTX 3080）

## 4. 要件（確定）

- **MVPの完成 = 新規作成〜編集〜.hayate保存/読込**。**PPTX入出力はMVP外**（次マイルストーン）
- **クロスプラットフォーム = 移植性重視・主テストはLinux**（gpuiのWindowsは成熟度に差あり）
- **譲れない非機能 = インストールの軽さ（モジュラー）+ UX**

## 5. アーキテクチャ（確定）

gpui依存をアプリ層に閉じ込め、コアをgpui非依存にする（テスト容易・移植性・ヘッドレス出力）。

```
hayate-ir       データ型のみ（serde, gpui非依存）              ← 文書モデル
hayate-model    編集状態 + 操作 + Undo/Redo（gpui非依存）       ← UXの核
hayate-format   Importer/Exporter トレイト + レジストリ
hayate-format-* 各フォーマット実装（hayate先行、pptxは後）       ← プラグイン
hayate-render   IR → 描画ディスプレイリスト(Scene)（gpui非依存）
hayate-app      gpui バインディング（View/入力/コマンドパレット）
```

主要な設計決定:

| 項目 | 決定 | 根拠 |
|---|---|---|
| 編集/Undo | **操作enum + 逆操作**（operation-centric、CRDT継ぎ目を用意） | Google(操作中心+OT)が成功、LibreOffice(可変モデルに後付け)が苦戦。土台は操作中心にし、CRDTは後からストレージ層に差し込める |
| プラグイン抽象 | **最初から汎用 `Document` 抽象** | 将来の表計算/ワープロを同じコアに乗せる |
| 描画境界 | **gpui非依存ディスプレイリスト**（`hayate-render`） | テスト容易・移植性・ヘッドレス出力 |
| ID/参照 | **slotmap（世代付き安定ID）** | 削除があっても選択/Undoが参照を保てる |

### 編集モデルの指針（補足）
- すべての変更を直列化可能な `Operation` として表し、状態変更は `apply(op)` の一点経路のみに集約する。
- Undo/Redo は逆操作をスタックに積む方式。
- 将来 automerge / yrs などのCRDTをバックエンドに差し込めるよう、操作中心の規律を最初から守る。

### 拡張機能システム（確定）
- **ランタイム = Rhai 中心**。純Rust・サンドボックス・外部ランタイム不要で、インストールの軽さ/移植性に合致。
  重い/多言語の拡張が必要になったら **WASM ABI を将来の継ぎ目**として追加する（最初はRhaiのみ）。
- **「AIによる拡張オーサリング」を最初から目玉UXとして設計**する。
  - 公開API（型・関数・`Operation`）を**機械可読スキーマ**として持ち、AIのコンテキストに渡す。
  - アプリ内で「自然言語 → 拡張生成 → サンドボックス実行 → エラーをAIに戻して再生成」の**自己修復ループ**を回す。
    これにより動的型付け・ニッチ言語の弱点（生成ミス）をユーザーに届く前に潰す。
  - Rhaiの典型パターンの**実例集**を few-shot として用意。
- 拡張の介入点（予定）: 操作(`Operation`)の発行、コマンド/メニュー/キーバインド追加、フォーマット変換、UIパネル。
- 設計上の含意: この「機械可読API/操作スキーマ」は §7-A の汎用 `Document` 抽象と表裏一体。
  拡張・AIが叩く面を、Rustのenumに直結させず**構造化コマンド＋スキーマのレジストリ**にすることで、
  Rhai生成・AI生成・将来のWASMの全てが同じ面を使える。

## 6. 実装状況

- `Cargo.toml`（ワークスペース定義）作成済み
  - ※現状 members は `hayate-ir` と `hayate-app` のみ。上記6クレート構成に後で更新が必要。
- gpui / gpui_platform を fork の rev ピンで参照する設定済み。

## 6.5 汎用 Document 抽象 + コマンドレジストリ（確定）

プレゼン/表計算/ワープロを **内部はドメイン特化・拡張境界は均一** の2層で扱う。

### 層1: `Document` トレイト（骨格・フォーマットレジストリが扱う）
```rust
pub trait Document: 'static {
    fn kind(&self) -> DocumentKind;                       // Presentation | Spreadsheet | Text | (拡張で追加可)
    fn apply(&mut self, op: Operation) -> Result<Operation>; // 逆操作を返す。状態変更の唯一の経路
    fn query(&self, q: &Query) -> QueryResult;            // 状態読み取り（AI/拡張用）
    fn snapshot(&self) -> Snapshot;                       // 保存・サムネ用
}
```
各文書型は自前の最適なデータ構造を内部に持つ（プレゼン=図形ツリー、表計算=グリッド、ワープロ=rope）。
共有プリミティブ（ID/座標/色/テキストrun/`Extensions`）は `hayate-ir` に集約。

### 層2: 構造化コマンド + スキーマレジストリ（拡張・AI・将来WASMが叩く面）
Rust enum に直結させず、`"presentation.move_shape"` のような **名前付き + 引数スキーマ** で登録する。

- **Command → Operation の2段**:
  - `Command`（=Blenderのoperator / VS Codeのcommand）: 名前付き・スキーマ付き・登録済みの「動詞」。
    パレット/キーマップ/Rhai/AI が叩く面。
  - `Operation`（=ProseMirrorのtransaction相当）: 直列化可能・**可逆**な状態差分。`apply(op)` が唯一の変更経路。
    ※具体形は §6.10 で「一様な4種（コンポーネントの Set/Remove/Spawn/Despawn）」に確定。
    Undoスタックが積むのはこちら。
  - 流れ: Command を呼ぶ → 1個以上の Operation を生成 → `Document.apply(op)` → 逆操作をUndoへ。
- **1スキーマ3役**（Blenderに倣う）: コマンドのプロパティスキーマから
  ①UI（コマンドパレット/プロパティフォーム） ②Rhai/AIの呼び出しシグネチャ ③Operationペイロード＋逆操作 を全部生やす。
- **宣言的manifest**（VS Codeに倣う）: 拡張は commands/menus/keybindings を manifest で宣言。
  コマンドパレットはレジストリから自動生成、`when`風の文脈ガードで出し分け（＝段階的開示UXの実装手段）。
- **in-place変更を禁止**（ProseMirrorに倣う）: 変更は必ず Operation 経由。これがUndoと将来CRDTの土台。

### 先行事例（独立分野が同じ構造に収束 → 定石と確認）
- Blender オペレータ: グローバル登録 + 型付きproperties(引数/Undo保存/UI自動生成の3役) + UI/keymap/Python共通呼び出し
- VS Code 貢献点: 宣言的manifest(commands/menus/keybindings) + `when`文脈 + パレット自動生成
- ProseMirror: Schema + Transaction(in-place禁止) + Transformが Undo と共同編集を可能にする

### MVPでの実装範囲（確定）
- Command/Operation レジストリの**仕組み**は最初から入れる。
- 実装する文書型は **Presentation 1型のみ**。Spreadsheet/Text は `DocumentKind` に予約するだけ。
- 根拠: レジストリの価値は「文書型の数」ではなく「コマンドの数」から来る（Blenderは1モデルに数百operator）。
  → 汎用性の利益を取りつつ MVP を肥大化させない。

## 6.6 テキスト / フォント / IME（確定）

決定的な事実: **gpuiは自分でテキストをshapeして描く**（`window.text_system().shape_line(text, font_size, &runs, None)`）。
外部shaperのグリフをgpuiで描くのは噛み合わないため、**テキスト整形はgpuiに委ねる**。

### 役割分担（テキストだけ整形がapp層）
| 層 | テキストの責務 |
|---|---|
| `hayate-ir` | モデル: `TextBody`→`Paragraph`→`Run{ font, size, color, bold, italic, underline }`、段落プロパティ（揃え・行間・箇条書きレベル）、`autofit`フラグ |
| `hayate-model` | 論理編集をOperation化（挿入/削除/書式）、**論理カーソル/選択**（段落index + byteオフセット、grapheme単位）。左右移動・編集・選択はここ（テスト可能） |
| `hayate-app` | gpuiで `shape_line`→描画、`EntityInputHandler` でIME/キー受領→Command/Operation化、ヒットテスト・IME候補窓位置・**自動縮小の倍率算出**。上下移動（折返し跨ぎ）とクリック位置決定はここ（レイアウト要） |

### IME（gpui `EntityInputHandler` を実装）
- `replace_and_mark_text_in_range` … 変換中の marked text（下線付き未確定）
- `replace_text_in_range` … 確定コミット
- `marked_text_range` / `unmark_text` … 変換中領域管理
- `bounds_for_range` … 候補ウィンドウ位置、`character_index_for_point` … ヒットテスト
- `ElementInputHandler::new(bounds, view)` で結線。marked領域は別runにして下線表示。

### Sceneでのテキスト
図形/画像は完全解決、**テキストだけは半抽象 `TextBlock`（矩形+段落+run+スタイル）** としてSceneに渡し、app層がshape+描画。

### MVPのリッチテキスト範囲（確定）
- run書式: font / size / color / 太字 / 斜体 / 下線
- 段落: 揃え（左/中/右）・行間・**箇条書き（1階層）**
- **テキストボックスの自動縮小（autofit）**
- 縦書きは将来課題（gpui/cosmic-textの対応が限定的で重い）

## 6.7 描画レイヤー Scene（確定）

`hayate-render` は「スライド1枚を指定ビューポートでpx解決した、描画＆ヒットテスト兼用の構造」を生成する。
gpui非依存。テキストだけ半抽象。

### 確認できたgpui描画API（ピン留めrev）
`paint_quad`（角丸+ボーダー）/ `paint_path`（塗り・線・破線）/ `paint_image` / `with_content_mask`（クリップ）/
`TransformationMatrix`（`rotate`/`scale`/`translate`/`compose`、quad・pathが`transformation`を持つ）/ グラデーション（`linear_gradient`+Oklab）。

### Scene 型（案）
```rust
pub struct Scene {
    pub viewport: Viewport,        // slide(EMU)→px 変換。逆変換で screen→slide
    pub background: Option<Paint>,
    pub nodes: Vec<SceneNode>,     // 並び順 = z順（グループは展開）
}
pub struct SceneNode {
    pub source: Option<NodeRef>,   // (SlideId, ShapeId) → ヒットテスト→選択
    pub transform: Affine,         // 回転等
    pub opacity: f32,
    pub clip: Option<ClipPath>,
    pub prim: Primitive,
}
pub enum Primitive {
    Quad  { bounds: RectPx, corner_radius: Px, fill: Paint, stroke: Option<Stroke> },
    Path  { path: PathData, fill: Option<Paint>, stroke: Option<Stroke> },
    Image { handle: ImageRef, bounds: RectPx },
    Text(TextBlock),               // 半抽象。app層が shape_line して描画
}
pub enum Paint { Solid(Rgba), LinearGradient { angle: f32, stops: Vec<(f32, Rgba)> } }
pub struct Stroke { color: Rgba, width: Px, dash: Option<Vec<Px>> }
```

### 確定事項
- **座標系 = px解決済み**。`render(slide, viewport) -> Scene`。`viewport` 保持で screen→slide 逆算しOperation発行。
- **ヒットテスト = Sceneを使う**（NodeRef付き・回転/楕円/パスでも幾何正確。描画とヒットテストが一貫）。
- **回転 = 全要素対応**（gpui `TransformationMatrix`）。
  ⚠️ テキスト/画像の回転描画はgpui APIへの適用方法を**実装時に検証**（ベクトル図形はpathに焼き込めば確実）。
- **更新戦略**: 編集時は表示中スライドのSceneのみ再生成。サムネはSceneをキャッシュし、変更スライドだけ再生成（仮想化＝大規模対応の布石）。

## 6.8 マスタースライド / 継承モデル（確定）

PowerPoint/OOXML/Google Slides 標準に準拠。§8.1-1（最重要seam）の具体化。

### 階層（3層継承）
**スライド → レイアウト → マスター**。各ページは自分が定義しない属性を上位から継承。
マスター1つに複数レイアウト（タイトル/コンテンツ等）。テーマは**マスター毎**（1デッキ内に複数テーマ可）。

```rust
struct Presentation {
    themes:  SlotMap<ThemeId, Theme>,      // 配色12色/フォント(major,minor)/効果
    masters: SlotMap<MasterId, SlideMaster>,
    layouts: SlotMap<LayoutId, SlideLayout>,
    slides:  SlotMap<SlideId, Slide>,
    slide_order: FracIndexMap<SlideId>,    // §8.1 fractional index
}
struct SlideMaster { theme: ThemeId, background: Fill, decorations: Vec<Shape>,
                     placeholders: Vec<Placeholder>, text_styles: OutlineTextStyles /*title/body9/other*/ }
struct SlideLayout { master: MasterId, name: String,
                     background: Option<Fill>, decorations: Vec<Shape>, placeholders: Vec<Placeholder> }
struct Slide       { layout: LayoutId, background: Option<Fill>, shapes: Vec<Shape>, notes: Option<String> /*…メタ枠*/ }
```

### 継承＝override表現（seamの実体） ※§6.10で改訂: `Option<T>`手通し → コンポーネント存在/不在で表現
- 継承可能な属性は **`Option<T>`（None=継承 / Some=上書き）**。
- プレースホルダは **`(type, idx)` で上位と照合**（Google Slides の parentObjectId 相当）。
- `Shape.placeholder: Option<PlaceholderRef{type, idx}>` で継承元を指す。

### 解決(resolve)
`hayate-model` に `resolve(slide) -> ResolvedSlide`（全属性が具体値）。チェーン Slide→Layout→Master→Theme を辿り
最初の `Some` を採用。**レンダラ/Sceneは解決済み値のみ**を見る（編集はoverride層を操作）。

### 描画順
`master.decorations` → `layout.decorations` → `slide.shapes`（背面→前面）。マスター装飾はスライドからは編集不可（背景図形非表示フラグで抑制可）。

### 編集セマンティクス（モード分離）
- **スライド編集モード**: プレースホルダに中身を入れる＝コンテンツ。位置/書式変更＝そのスライドに `Some` override（1枚だけ変わる）。
- **マスター編集モード**: マスター/レイアウトを直接編集＝継承する全スライドに波及。
- 「1枚だけ」か「全部」かをモードで明確化（PowerPoint方式）。

## 6.9 .hayate フォーマット / 保存パイプライン（確定）

§8.1-5 の具体化。決定軸は「ストレージエンジン」のみ、他（構造/シリアライズ/passthrough/WAL）は直交。

### ストレージエンジン = redb（純Rust・ACID 組み込みDB）
理由: 差分更新・原子性をネイティブで得つつ**純Rust**（install軽さ・クロス容易の方針に最合致）。
SQLite（C依存だが成熟・SQL）/ ZIP（開かれた形式・OOXML流儀だが差分不可）と比較の上で採用。
クエリ層が無い点は「IRはメモリ保持・ロジックはRust側」なので不要。検証性は**JSONダンプ用デバッグコマンド**で補う。

### 構造（redbのテーブル/キー）
- `slides`（key=SlideId, value=slide JSON） … **per-slide**（§確定）。遅延読込＝1キー読み。
- `masters` / `layouts` / `themes` … マスター継承構造（§6.8）。
- `media`（key=内容ハッシュ, value=BLOB） … 画像/動画/音声。重複排除。
- `fonts`（埋め込みサブセット枠、後）。
- `oplog` … 操作ログ（Undo/版履歴/復旧の源）。
- `manifest` … format_version, app_version, doc_kind, 索引, slide_order(fractional index)。

### シリアライズ = JSON（passthrough付き）
各値はJSON。未知フィールドは `#[serde(flatten)] extra` に退避＝**前方互換（lossless escrow）**。
速度/サイズが問題化したら将来バイナリ列を追加可能（直交）。

### 原子性・WAL・復旧
- **保存＝redbトランザクション**（ACID）。自前のtemp→renameは不要、クラッシュ安全。
- 編集中の操作は `oplog` に追記（＝WAL）。operation-centric編集と直結。
- クラッシュ後の再起動で「未保存編集の復元」を提示（oplogを最終スナップショットに再適用）。

### 暗号化（枠のみ予約・MVP外）
値単位の暗号化が差分更新と両立しやすい（各valueを暗号化して格納、索引キーは平文）。
丸ごと暗号化が要るときは全体ラップ。AES-256 + Argon2 は後。

### 前方互換ポリシー
manifest `format_version`（semver）。minor新しい=警告付き読込（passthroughで無損失）/ major新しい=読み取り専用。

## 6.10 ドキュメントモデル: DOD 自前列ストア（確定・§6.5/6.8/6.9を改訂）

データ指向設計(DOD)を**コアのドキュメントモデルにのみ**適用（UIはgpui反応型モデル据え置き＝研究上ECSはGUIに不向き）。
コード比較の結果、フルECSライブラリ(hecs/bevy_ecs)はコンポーネントがRust静的型必須で**実行時プラグインが新データ型を定義できず衝突**するため、**自前の列ストア**を採用。

### World（エンティティ＋コンポーネント列）
```rust
slotmap::new_key_type! { pub struct Entity; }
#[derive(Default)] pub struct World {
    alive:   SlotMap<Entity, ()>,
    frames:  SecondaryMap<Entity, RectEmu>,    // コンポーネント＝独立した疎な列
    fills:   SecondaryMap<Entity, Fill>,       // 列に値が無い = 継承/未設定
    texts:   SecondaryMap<Entity, TextBody>,   // ※深い入れ子(段落/run)は1コンポーネント値の中の構造体。run毎にエンティティ化しない
    parent:  SecondaryMap<Entity, Entity>,     // 階層は親参照コンポーネント
    order:   SecondaryMap<Entity, FracIndex>,  // 兄弟順序（§8.1 fractional index）
    dynamic: HashMap<CompTypeId, Box<dyn ComponentColumn>>, // 実行時登録の動的列（プラグイン用）
}
```

### Operation = 一様な4種（§6.5の「プロパティ毎op」を置換）
```rust
pub enum Operation {
    SetComponent    { entity: Entity, comp: CompValue },  // 無→有 含む置換
    RemoveComponent { entity: Entity, ty: CompTypeId },
    Spawn   { entity: Entity },
    Despawn { entity: Entity },
}
impl Operation { fn apply(self, w:&mut World) -> Operation /*inverse*/ }
```
- **利点**: undo/直列化(oplog/WAL)/将来CRDT が扱う形が4種に固定。プロパティが増えてもop型は増えない。
- Commandレジストリ(§6.5)は不変: 名前付き・スキーマ付きの「動詞」はそのまま。Commandハンドラがこの4 opを生成する。
  「閉じた語彙 vs 開いた拡張」の緊張は解消（**形は4種で閉、内容はコンポーネント追加で開**）。

### 各設計への反映
- **継承override(§6.8)** = `Option<T>`手通しを廃し、**コンポーネントの存在/不在＝override/継承**（疎に表現）。`resolve` は列を slide→layout→master と辿る。
- **プラグイン拡張(§6.5逃げ道)** = `SetExtData` のJsonValueバッグを廃し、**実行時登録の型付き動的列＋スキーマ**（クエリ可能）。
- **永続化(§6.9)** = redbの**テーブル＝コンポーネント列**として自然対応。
- **描画/レイアウト/ヒットテスト** = 必要な列だけ連続走査（キャッシュ効率）。

### コスト/緩和
1図形のデータが列に分散しデバッグしにくい → 「エンティティの全コンポーネントを集めて1 structにダンプする」デバッグ表示を用意。

## 6.11 拡張のケイパビリティ関門（確定・§8.1-9 / §5の具体化）

拡張（今Rhai／将来WASM）がドキュメント・ホストに触れるのは **`HostApi` 一点経由**のみ。
権限と資源をここで強制し、Rhai⇄WASM 載せ替えもこの抽象の裏に閉じ込める。

### マニフェスト（VS Code流）
```rust
struct ExtManifest { id, version,
    contributes: Contributes,       // commands/menus/panels/動的コンポーネント — 宣言的・低リスク
    capabilities: Vec<Capability> } // 要求権限 — ユーザー同意が要る（deny-by-default）
enum Capability {                   // ★細かいスコープ付き（WASI風）
    DocumentRead, DocumentWrite,
    Filesystem { read: Vec<Glob>, write: Vec<Glob> },
    Network { domains: Vec<String> }, Clipboard, Ai, ProcessExec }
```

### HostApi（唯一のチョークポイント）
- `invoke(id, args)` … 登録コマンド実行（高水準・undo単位）
- `query(q)` … 読み取り（`DocumentRead` 要）
- `emit(ops)` … **4種opの直接発行（`DocumentWrite` 要）** ← 書込経路は「コマンド経由 + 権限付きop発行」の両対応
- `fs_read/http_get/…` … 各 `require(capability)` を通過した時のみ
- 「現在実行中の拡張のGrantSet」を基準に `require` で判定。

### 強制（Rhai今 / WASM後）
- **Rhai**: 既定サンドボックス的。関門＝**付与権限に対応するホスト関数だけEngineに登録**（未付与は生やさない）。
  資源は `set_max_operations`（命令数）/`on_progress`（タイムアウト）/呼び出し深さ・文字列/配列サイズ上限。
  ※同一プロセスゆえメモリのハード隔離は無い（op数・時間で緩和）。
- **WASM(後)**: wasmtime fuel/epoch（CPU）+ ResourceLimiter（メモリ）+ WASI capability（fs/net）。ホスト関数群は同じ `HostApi` に委譲。
- 実行は**ワーカースレッド＋キャンセル境界**（ハングした拡張でUIを固めない）。

### 信頼レベル
- 組込/一次拡張＝全権。ユーザー導入＝導入時に要求権限を提示し同意（モバイルアプリ流、スコープ提示）。
- **文書埋め込みスクリプト＝既定で無効/無権限**（開く＝コード実行のマクロウイルス回避。実行は明示有効化が必要）。
- AI生成拡張も未信頼扱い・明示付与・サンドボックス内で自己修復ループ。

## 6.12 アクセシビリティ（a11y）層（確定・§8.1-8）

スクリーンリーダー（NVDA/VoiceOver等）が画面内容を**合成音声で読み上げる**ための構造（アクセシビリティツリー）を提供する。
法的要件でもあり（Section 508 / EN 301 549 / JIS X 8341）、OSS Officeが弱い領域＝差別化機会。

### 確認できた事実（pin rev）
gpuiは **AccessKit を統合済み**（`accesskit.workspace=true`、`_accessibility.rs`、`a11y.rs` example）。
要素ビルダーに `.role(Role::…)` / `.aria_label()` / `.aria_*()` / `.on_a11y_action(...)` があり、
AccessKit が AT-SPI(Linux)/UIA(Win)/NSAccessibility(Mac) へ橋渡し。フォーカスは `FocusHandle`。

### 課題: 自前描画の中身
UIクローム（ツールバー/パネル/パレット/一覧）は gpui 要素なので `.role()/.aria_label()` で**ほぼ無料**。
だが**スライド本体は canvas 描画**＝スクリーンリーダーには「画像」一枚にしか見えない。図形/テキストを個別にツリー化する必要がある。

### 確定方針
- **エンティティに a11y コンポーネントを予約**（`role` / `label`(=alt) / `description` / `reading_order` / 値）。§8.1-10 の alt/読み上げ順がここで効く。
- **gpui非依存の `A11yTree` を core で生成**（Scene と同じ発想: モデル→バックエンド非依存記述→app が AccessKit へ写像）。読み上げ順は z順でなく `reading_order`。
- **1ツリー2用途**: `A11yTree` を画面読み上げ(AccessKit)と **PDF/UA タグ付き出力**の両方に使う（二重実装回避）。
- **MVPスコープ**: ①a11yコンポーネントの席を今予約 ②**クロームa11yはMVPで実装**（gpui要素に role/label）③**スライド中身の読み上げ（透明オーバーレイ or AccessKit仮想ノード）は MVP後**（gpuiのcanvas a11y対応は実装時に検証）。

## 6.13 Command スキーマ機構（確定・§6.5の深掘り）

「1スキーマ3役（AI tool / UIフォーム / Operation生成）」の具体化。正準表現は **JSON Schema に統一（ハイブリッド）**。

### 正準表現＝JSON Schema
```rust
pub struct CommandSpec {
    id: CommandId, meta: CommandMeta,           // title/category/icon/when
    params: schemars::Schema,                   // JSON Schema：AI tool / UIフォーム / 検証 の源
    handler: Box<dyn Fn(&mut Ctx, Value) -> Result<Vec<Operation>>>,
}
```
- **組込コマンド**: 型付き `Args`(derive `Deserialize`+`JsonSchema`) で登録 → `schema_for!` でJSON Schema自動生成、
  handlerは内部で `Value→Args` に serde 変換してから型付きクロージャを呼ぶ（人間工学）。
- **プラグインコマンド**（Rhai/WASM・実行時登録）: JSON Schema を**実行時データ**として供給、handlerは `Value` を動的に受ける。
  → Rustのderiveが使えないプラグインと、型安全な組込を、同一の `CommandSpec` に統一。

### 呼び出し経路（検証の関門）
```
invoke(id, args: Value, ctx)
 → spec = registry[id]
 → validate(args, spec.params)   // JSON Schema検証で AI/Rhai/動的型の誤りを弾く
 → spec.handler(ctx, args)       // 組込は内部でArgsへdeserialize
 → Vec<Operation> を 1 Transaction で apply（§6.10）
```

### 1スキーマ3役
- **AI**: `registry.manifest()` → `[{id, title, description, parameters: <JSON Schema>}]` ＝ **AIのtool定義そのもの**。
  生成ミスは上の検証で弾き、§5の自己修復ループへ。
- **UI**: JSON Schema からコマンドパレット項目＋プロパティ入力フォームを自動生成。
- **Operation**: handlerが型付き(組込)/動的(プラグイン)に §6.10 の4種opを生成。

### なぜ JSON Schema か
AIのtool-use APIが食う形そのもの＝AI連携が無加工。`schemars` クレートでRust型から自動導出。

## 6.14 テーマ / トークン（確定・§8.1-2 / §6.8テーマ毎 / §6.10 Color=コンポーネント）

OOXML標準準拠。色/フォントを**リテラルでなくテーマ参照**で持ち、テーマ一括変更で全スライド連動。

### 表現（§6.10のコンポーネント値）
```rust
enum Color {
    Literal(Rgba),
    Theme { token: ThemeColorToken, xf: ColorXf },   // 参照色 + 変換
}
enum ThemeColorToken { Dk1, Lt1, Dk2, Lt2, Accent1, Accent2, Accent3, Accent4, Accent5, Accent6, Hlink, FolHlink }
struct ColorXf { lum_mod, lum_off, tint, shade, alpha, sat_mod }  // 恒等がデフォルト。明暗バリエーションを表現

enum FontRef { Family(String), Theme(ThemeFontSlot) }   // ThemeFontSlot = Major | Minor
struct Theme {
    colors: [Rgba; 12],                                  // 12トークン
    fonts: FontScheme { major: ScriptFonts, minor: ScriptFonts },
}
struct ScriptFonts { latin: String, ea: String, cs: String }  // ea=日本語等。latin+ea+cs の3スクリプト対応
```

### 解決（DODの「システム」、Sceneビルダーで実行）
- `resolve_color(Color, theme) -> Rgba`: Literalはそのまま / Themeはトークン参照→`xf`適用。
- `resolve_font(FontRef, theme, script) -> Family`: runのスクリプト（日本語=ea）でmajor/minorのスロットを引く。
- レンダラ(§6.7)はリテラル値のみ見る。テーマ参照はマスター経由（§6.8 `slide.layout.master.theme`）。

### 確定スコープ
- **配色 = 12色 + 変換**（tint/shade/lumMod等）。カラーピッカーの明暗バリエーションを表現可能。
- **clrMap（bg1/tx1/bg2/tx2 の意味マッピング）は枠のみ予約**（PPTX忠実度向け、実装は後）。
- **フォント = latin+ea+cs のスクリプト別**（日本語と欧文で別フォント指定可。日本語品質に直結）。
- **モード（Light/Dark等、Figma流）は将来の一般化として席のみ**。MVPはマスター毎テーマで足りる。

## 6.15 アニメーション / タイムライン（確定・§8.1-11）

### データモデル（簡易ステップ列）
OOXMLのSMILタイミング木でなく、実用95%をカバーする**ステップ列**を採用。
```rust
struct SlideTimeline { steps: Vec<AnimStep> }            // スライドのDODコンポーネント
struct AnimStep { trigger: Trigger, anims: Vec<Anim> }   // step内は並行再生
enum Trigger { OnClick, WithPrev, AfterPrev { delay: Ms } }
struct Anim { target: Entity, kind: AnimKind, duration: Ms, delay: Ms, easing: Easing }
enum AnimKind { Entrance(Effect), Emphasis(Effect), Exit(Effect), MotionPath(PathData) }
struct Transition { kind: TransitionKind, duration: Ms, direction: Dir }  // 画面切替：slideに付与
```

### Morph / Magic Move
差別化の目玉。shapeに **`morph_key`（スライド複製時に継承する照合キー）を予約** → 隣接スライドで同keyをペアし
位置/サイズ/回転/色を自動補間。安定ID(§6.10)＋明示キーで確実に照合。

### 再生と描画
タイムライン=データ、再生=時間駆動エンジン。Sceneを**時刻パラメータ付き**生成(§6.7)すれば
再生・Morph補間・動画書き出し(§Gオフスクリーン)が同一経路。
**MVP = データモデルと `morph_key` を予約、再生エンジンはMVP後**。

## 6.16 テキストレイアウト中核（確定・§8.1-7 / §6.6・§8.2の具体化）

### 責務分担
```rust
// core（gpui非依存・テスト可能）
enum WritingMode { HorizontalTb, VerticalRl, VerticalLr }  // 一級市民。横書きハードコード禁止
trait LineBreaker {                                        // 差し替え可能な行分割ポリシー
    fn break_points(&self, runs: &[MeasuredRun], width: Px, wm: WritingMode) -> Vec<Break>;
}
//   実装: DefaultUax14（汎用UAX#14）/ Japanese（UAX#14 + 禁則 JIS X 4051）
```
- **グリフ整形・測定 = app（gpui `shape_line`）**: 合字/フォールバック/字形。
- **行分割の判断 = core `LineBreaker`**: 禁則はここ。測定済み幅を入力に純ロジック＝テスト可能。
- **行の積み方 = writing-mode**: 横書き=上→下、縦書き=右→左。座標変換に writing-mode を通す。

### 確定スコープ
- **MVP = 基本禁則を実装**（行頭禁則: 。、」）等／行末禁則: 「（等）。日本語品質の差別化。`Japanese` LineBreaker で。
- **writing-mode は一級市民として席を予約**、MVPは `HorizontalTb` のみ実装。縦書き/RTLは後（横書きハードコードは§8.2で禁止）。
- bidi(RTL)も同じ書字方向抽象に内包（後）。

## 8. 競合調査による要件カタログ（要点）

PowerPoint/Slides/Keynote/Canva/Figma/LibreOffice/OnlyOffice 等を調査。全量は `docs/REQUIREMENTS.md`。
ここには「設計に直接効く2点」＝①今すぐ予約すべき継ぎ目 ②既存決定の見直し点 だけを置く。

### 8.1 今すぐ予約すべき早期アーキの継ぎ目（後付けが高コストな順）
これらは「実装は後でも、データモデル/境界の"席"だけは最初に用意する」。後付けは破壊的変更になる。
1. **マスター/レイアウト/プレースホルダの継承カスケード** — プレゼンの背骨。`Presentation` を「マスター→レイアウト→スライド」継承で設計。
2. **色/フォント/値の「テーマ・トークン参照」間接層** — `Run`/`Shape` の色やフォントを*リテラルでなく参照可能*に（mode/テーマ一括変更の源）。
3. **安定グローバルID + コンポーネント/インスタンス+オーバーライド差分** — slotmap安定ID（既存方針）を全ノードに。再利用部品は「定義参照+差分」。
4. **兄弟順序の fractional indexing** — スライド/図形の順序を `Vec` index でなく0–1の分数キーで（Undo/将来CRDT/Morphの同一性が安定）。→ §8.2で要判断。
5. **保存パイプライン単一化** — シリアライズ→(任意)暗号化→**atomic write**（temp→fsync→rename）。+ 操作ログを**WAL**にして自動保存/復旧/版履歴を同一基盤に。
6. **ヘッドレス/オフスクリーン・レンダリング経路** — 「UIを介さず IR→出力」。PDF/画像/動画書出・サムネ・テスト・変換CLIの共通前提（`hayate-render` のgpui非依存方針と整合）。
7. **テキストレイアウト中核** — シェーピング+フォント解決/フォールバック+**CJK禁則**+**writing-mode（縦書き/RTL）**。→ §8.2で要判断。
8. **a11yセマンティクス層（AccessKit前提）** — gpuiは自前描画ゆえa11yは無料で付かない。UI要素が Role/Name/Value/フォーカスを表明するtrait。
9. **拡張ホストのケイパビリティ関門** — 拡張に渡すAPI集合=権限を1箇所に集約（Rhai→WASMサンドボックス載替可能に）。
10. **スライドのメタ枠を最初から予約** — ノート/表示タイミング/alt text/読み上げ順/ハイパーリンク・アクション/セクション/非表示フラグ/アニメーション。フィールド確保は安く、後付けはフォーマット互換を壊す。
11. **アニメーション・タイムライン + スライド間オブジェクト同一性追跡** — Morph/Magic Move差別化の前提（安定IDに依存）。
12. **発表者ビュー=二系統出力**（聴衆用/発表者用の独立レンダリング）+ ドキュメント=順序付きページ集合、**論理座標/スライド実寸(pt)/物理px の分離**、画像の原本/表示テクスチャ分離、スライド単位の遅延実体化。

### 8.1-決定 seam予約の方針（確定）
**MVP IR は §8.1 の12項目すべての seam（型の席）を予約する**（実装は段階的、席は今）。
将来の手戻り（データ移行・大量書き換え）を最小化する方針。これに伴い §8.2 の微修正も全採用。

具体的に MVP IR に織り込む席:
- マスター/レイアウト/プレースホルダ継承の構造（中身は最小実装、継承解決は後）
- `Color`/`Font` を `Literal | ThemeRef` の二者対応に（実装は Literal のみ）
- 全ノードに安定ID（slotmap）、再利用部品の「定義参照+オーバーライド差分」枠
- スライド/図形の順序を **fractional index**（`Vec` index廃止）
- 保存パイプライン: シリアライズ→(任意)暗号化枠→atomic write、操作ログWAL枠
- レンダリングを画面/オフスクリーン両対応の入口に（`hayate-render` gpui非依存）
- テキスト: グリフ整形=app層gpui、**行分割/writing-modeポリシー=core側trait**（縦書きは席のみ）
- a11y: UI要素が Role/Name/Value/フォーカスを表明する trait の席
- 拡張: ホストが渡すAPI=ケイパビリティ関門を1箇所に
- スライドのメタ枠: notes / timing / alt / 読み上げ順 / hyperlink・action / section / hidden / animation
- アニメーション・タイムライン枠 + スライド間オブジェクト同一性（安定IDで担保）
- ドキュメント=順序付きページ集合、論理座標/スライド実寸(pt)/物理px分離、画像の原本/表示分離、遅延実体化の余地

### 8.2 既存決定への影響・再検討事項（→ 上記方針で全採用）
調査で、これまでの決定を**補強 or 微修正**すべき点が出た:
- **順序表現（§5/§6.5の補足）**: 既存は slotmap + `slide_order: Vec<SlideId>`。Figma事例より、**fractional index** にすると Undo/CRDT/Morph が安定。MVPでもコスト小なので採用を推奨（要確認）。
- **テキスト整形の境界（§6.6の補足）**: 「整形はapp層のgpui text_system」は維持。ただし **CJK禁則・縦書き・bidi のポリシー層はgpuiが面倒を見ない可能性**。グリフ整形=gpui、**行分割/書字方向ポリシー=core側の差し替え可能な層**、という二段に分けるのが安全（縦書きは将来でも、writing-mode抽象の"席"は今）。
- **色/フォントの保持（§6.6/§6.7の補足）**: `Run`/`Paint` を「リテラル or テーマ参照」の二者に対応させる（§8.1-2）。MVPはリテラルのみ実装でも、型に参照ケースの席を用意。
- **.hayateフォーマット（要件§O/§N）**: 「スナップショット+操作ログ」前提＋atomic write＋暗号化コンテナの差込口＋セクション境界/チェックサムを設計に織り込む。
- **ライセンス（要件§S）**: 将来OOXML/ODF実装やAGPLコードを取り込む際の汚染回避にプロセス境界の余地。gpui fork(Apache系)の互換も確認。

## 9. 未決・議論中のトピック

- ~~A. 汎用 `Document` 抽象の具体形~~ → §6.5 に確定済み
- ~~拡張機能（extension）システム~~ → §5「拡張機能システム」に確定済み
- ~~B. テキスト/フォント・IME~~ → §6.6 に確定済み（§8.2で禁則/縦書きの補足あり）
- ~~D. `hayate-render` の Scene 表現~~ → §6.7 に確定済み
- 実装ロードマップ（M0〜）が未策定。Cargo.toml を6クレート構成へ更新が必要。
- §8.2 の再検討事項（fractional index / writing-mode層 / 参照色）を MVP IR に織り込むか要判断。
- C. `Operation` の粒度とコマンドパレット/キーバインドのAPI
- D. `hayate-render` の Scene 表現（描画プリミティブの設計）
